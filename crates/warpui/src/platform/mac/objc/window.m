#import <AppKit/AppKit.h>
#import <AppKit/NSAccessibility.h>
#import <AppKit/NSAccessibilityConstants.h>
#import <UniformTypeIdentifiers/UniformTypeIdentifiers.h>
#import <objc/runtime.h>

#import "alert.h"
#import "app.h"
#import "fullscreen_queue.h"
#import "host_view.h"
#import "window_blur.h"

// NSWindow.delegate is a weak reference, so the WarpWindowDelegate we create in
// `create_warp_nswindow` / `create_warp_nspanel` would otherwise be leaked with a +1
// retain count. Associating it with the window ties its lifetime to the window: the
// associated object is released by the runtime when the window itself is deallocated.
static const void *kWarpWindowDelegateAssocKey = &kWarpWindowDelegateAssocKey;

NSWindowStyleMask warpWindowMask = NSWindowStyleMaskClosable | NSWindowStyleMaskMiniaturizable |
                                   NSWindowStyleMaskResizable | NSWindowStyleMaskTitled;

// The default macOS titlebar height (in points).
static const CGFloat DEFAULT_TITLEBAR_HEIGHT = 28.0;
static const NSSize MIN_WINDOW_SIZE = {480.0, 192.0};
static const NSSize TEST_MIN_WINDOW_SIZE = {124.0, 34.0};

// A back-to-front ordered array of windows, identified by their `windowNumber`
// property.
NSMutableArray<NSNumber *> *windowOrderForTests;
dispatch_once_t windowOrderOnce;

FullscreenWindowManager *fullscreenManager;
dispatch_once_t fullscreenQueueOnce;

// This extends the NSWindow API with an implementation of toggleFullScreen
// that enforces one window transition at a time, preventing concurrent
// animations.
@interface NSWindow (Fullscreen)
- (void)enqueueFullscreenTransition;
@end

@implementation NSWindow (Fullscreen)
- (void)enqueueFullscreenTransition {
    // If the queue doesn't already exist, allocate it.
    dispatch_once(&fullscreenQueueOnce, ^{
      fullscreenManager = [[FullscreenWindowManager alloc] init];
    });

    // Enqueue the window into the fullscreen manager asynchronously, to ensure
    // there are no synchronous callbacks into Rust code.
    dispatch_async(dispatch_get_main_queue(), ^{
      [fullscreenManager enqueueWindow:self];
    });
}
@end

@protocol WarpWindowProtocol

@property BOOL testMode;

@property BOOL hideTitleBar;

// Asynchronously marks the content view as being dirty.
- (void)setNeedsDisplayAsync;

// Configures the titlebar height and traffic light button constraints.
- (void)configureTitlebarHeight:(CGFloat)height;

// Resets the titlebar height to the default macOS value for fullscreen. Fullscreen has a different
// titlebar which cannot honor user-configured height.
- (void)applyFullscreenTitlebarHeight;

// Restores the titlebar height to the last value passed to configureTitlebarHeight:.
- (void)restoreConfiguredTitlebarHeight;

@end

@class WarpWindow;
@class WarpPanel;

// Declaration of functions implemented in Rust.
void warp_dealloc_window(id self);
void warp_dispatch_standard_action(id self, NSInteger tag);
void warp_app_window_moved(id self, NSRect rect);
void warp_open_panel_file_selected(id urls, void *callback);
void warp_save_panel_file_selected(id url, void *callback);

NSNumber *previouslyActiveAppPID;

@interface PreviousStateHelper : NSObject
@end

@implementation PreviousStateHelper
+ (NSNumber *)storePreviousState {
    NSRunningApplication *runningApp = [[NSWorkspace sharedWorkspace] frontmostApplication];
    NSString *bundleIdentifier = runningApp.bundleIdentifier;
    if ([bundleIdentifier isEqualToString:[[NSBundle mainBundle] bundleIdentifier]]) {
        return nil;
    } else {
        return @(runningApp.processIdentifier);
    }
}

+ (void)activatePreviousState:(NSNumber *)previousPID {
    if (previousPID) {
        NSRunningApplication *app =
            [NSRunningApplication runningApplicationWithProcessIdentifier:[previousPID intValue]];
        if (app) {
            // Use the default behavior here which only activates the main and key window.
            [app activateWithOptions:(NSApplicationActivateAllWindows |
                                      NSApplicationActivateIgnoringOtherApps)];
        }
    }
}
@end

@interface WarpWindow : NSWindow <WarpWindowProtocol>
@end

@interface WarpWindowDelegate : NSObject <NSWindowDelegate>
@end

@implementation WarpWindowDelegate {
    void *windowState;

    BOOL forceTermination;
}

- (void)windowDidMove:(NSNotification *)notification {
    if (windowState) {
        NSWindow *window = notification.object;
        warp_app_window_moved(self, window.frame);
    }
}

- (void)windowWillStartLiveResize:(NSNotification *)notification {
    WarpWindow *warp_window = notification.object;
    WarpHostView *warp_view = warp_window.contentView;

    // This is a hack to get around `borrowMut` errors within the UI framework
    // caused by the fact that it incorrectly assumes that callbacks cannot
    // synchronously cause another callback to be triggered. To avoid this for now,
    // we explicitly force callbacks to be synchronous if it's caused by the user instead
    // of another system call (such as the active screen changing)
    [warp_view setAsyncCallback:NO];
}

- (void)windowDidEndLiveResize:(NSNotification *)notification {
    WarpWindow *warp_window = notification.object;
    WarpHostView *warp_view = warp_window.contentView;
    [warp_view setAsyncCallback:YES];
}

- (void)setForceTermination {
    forceTermination = YES;
}

- (BOOL)windowShouldClose:(NSWindow *)window {
    if (forceTermination) {
        return YES;
    }

    NSApplication *application = [NSApplication sharedApplication];
    BOOL okToClose = warp_app_should_close_window(application, window);

    if (okToClose) {
        return YES;
    } else {
        return NO;
    }
}

- (void)windowWillClose:(NSNotification *)note {
    if (windowState) {
        warp_app_window_will_close([NSApplication sharedApplication], self);
    }
}

- (NSApplicationPresentationOptions)window:(NSWindow *)window
      willUseFullScreenPresentationOptions:(NSApplicationPresentationOptions)proposedOptions {
    return proposedOptions | NSApplicationPresentationAutoHideToolbar;
}

- (void)windowWillEnterFullScreen:(NSNotification *)notification {
    NSWindow<WarpWindowProtocol> *window = notification.object;
    [window applyFullscreenTitlebarHeight];
    // macOS automatically detaches the title bar in fullscreen (see
    // willUseFullScreenPresentationOptions), and shows it along with the mac menu on hover. Since
    // the title bar is overlaid in this case, it should be visible.
    window.titlebarAppearsTransparent = NO;
}

- (void)windowWillExitFullScreen:(NSNotification *)notification {
    NSWindow<WarpWindowProtocol> *window = notification.object;
    window.titlebarAppearsTransparent = window.hideTitleBar;
    [window restoreConfiguredTitlebarHeight];
}

@end

// Returns the titlebar container view for the given window, or nil if not found.
static NSView *get_titlebar_container_view(NSWindow *window) {
    NSButton *closeButton = [window standardWindowButton:NSWindowCloseButton];
    if (!closeButton) return nil;
    NSView *titleBarView = [closeButton superview];
    return [titleBarView superview];
}

// Configures titlebar height and traffic light button constraints for a window.
// Returns the height constraint if newly created, or NULL if just updating.
static NSLayoutConstraint *configure_titlebar_height(NSWindow *window, CGFloat height,
                                                     NSLayoutConstraint *existingConstraint) {
    if (height <= 0) {
        return existingConstraint;
    }

    NSView *titleBarContainerView = get_titlebar_container_view(window);
    if (!titleBarContainerView) {
        return existingConstraint;
    }
    NSView *titleBarView = [titleBarContainerView.subviews firstObject];
    if (!titleBarView) {
        return existingConstraint;
    }

    // Set title bar container's height and origin.
    NSRect containerFrame = [titleBarContainerView frame];
    CGFloat windowHeight = [window frame].size.height;
    containerFrame.size.height = height;
    containerFrame.origin.y = windowHeight - height;
    [titleBarContainerView setFrame:containerFrame];

    // Edit existing constraint if already constructed.
    if (existingConstraint) {
        existingConstraint.constant = height;
        return existingConstraint;
    }

    // Otherwise, we're building for the first time.
    titleBarView.translatesAutoresizingMaskIntoConstraints = NO;

    NSLayoutConstraint *heightConstraint =
        [titleBarView.heightAnchor constraintEqualToConstant:height];
    heightConstraint.priority = NSLayoutPriorityRequired;
    heightConstraint.active = YES;

    // Pin titlebar to top, left, and right of container.
    [[titleBarView.topAnchor constraintEqualToAnchor:titleBarContainerView.topAnchor]
        setActive:YES];
    [[titleBarView.leadingAnchor constraintEqualToAnchor:titleBarContainerView.leadingAnchor]
        setActive:YES];
    [[titleBarView.trailingAnchor constraintEqualToAnchor:titleBarContainerView.trailingAnchor]
        setActive:YES];

    NSButton *closeButton = [window standardWindowButton:NSWindowCloseButton];
    NSButton *miniaturizeButton = [window standardWindowButton:NSWindowMiniaturizeButton];
    NSButton *zoomButton = [window standardWindowButton:NSWindowZoomButton];

    if (!closeButton || !miniaturizeButton || !zoomButton) {
        return heightConstraint;
    }

    // Standard macOS traffic light button spacing.
    CGFloat buttonSpacing = 6.0;
    CGFloat leftMargin = 12.0;
    CGFloat buttonSize = 14.0;

    NSArray *buttons = @[ closeButton, miniaturizeButton, zoomButton ];
    for (NSUInteger i = 0; i < buttons.count; i++) {
        NSButton *button = buttons[i];
        button.translatesAutoresizingMaskIntoConstraints = NO;

        [[button.widthAnchor constraintEqualToConstant:buttonSize] setActive:YES];
        [[button.heightAnchor constraintEqualToConstant:buttonSize] setActive:YES];

        CGFloat xOffset = leftMargin + i * (buttonSize + buttonSpacing);
        [[button.leadingAnchor constraintEqualToAnchor:titleBarView.leadingAnchor
                                              constant:xOffset] setActive:YES];
        [[button.centerYAnchor constraintEqualToAnchor:titleBarView.centerYAnchor
                                              constant:1.0] setActive:YES];
    }

    return heightConstraint;
}

// Initializes an NSWindow that conforms to our window protocol.
void init_warp_nswindow(NSWindow<WarpWindowProtocol> *window, bool testMode, bool hideTitleBar) {
    window.testMode = testMode;
    window.hideTitleBar = hideTitleBar;
    NSSize minWindowSize = testMode ? TEST_MIN_WINDOW_SIZE : MIN_WINDOW_SIZE;

    // Set the background color to clear to support window background transparency. When this is set
    // to NSColor.clearColor with alpha = 0 and window drop shadows are enabled, MacOS renders a
    // small 'gap' between the window border and the contents.  We don't know why; its likely an
    // internal Cocoa bug. https://stackoverflow.com/questions/6167692/nswindow-shadow-outline
    // provides evidence that we're not the only one observing this issue.
    //
    // Setting some non-zero alpha component for the background color fixes the issue.
    window.backgroundColor = [NSColor.clearColor colorWithAlphaComponent:0.01];
    window.releasedWhenClosed = YES;
    window.acceptsMouseMovedEvents = YES;
    window.titlebarAppearsTransparent = hideTitleBar;
    window.titleVisibility = hideTitleBar ? NSWindowTitleHidden : NSWindowTitleVisible;
    window.minSize = minWindowSize;
    window.contentMinSize = minWindowSize;
    if ([window respondsToSelector:@selector(setMinFullScreenContentSize:)]) {
        window.minFullScreenContentSize = minWindowSize;
    }
}

@implementation WarpWindow {
    // The windowState is managed on the Rust side.
    void *windowState;
    // Height constraint for the titlebar view (also indicates if constraints are configured)
    NSLayoutConstraint *_titleBarHeightConstraint;
    // The last height set via configureTitlebarHeight: (i.e. from Rust).
    CGFloat _configuredTitlebarHeight;
    // Whether we have registered for titlebar container frame change notifications. Needed to
    // uphold the user-configured titlebar height.
    BOOL _observingTitlebarContainer;
    // Guard to prevent re-entrancy when we change the titlebar container frame ourselves.
    BOOL _isApplyingTitlebarHeight;
    // When YES, constrainFrameRect:toScreen: returns the requested frame unmodified. This prevents
    // macOS from cascading or clamping the window position while a tab-drag preview window is
    // being created and positioned under the cursor.
    BOOL _suppressFrameConstraintsDuringDrag;
}

@synthesize testMode;
@synthesize hideTitleBar;

- (void)applyTitlebarHeight:(CGFloat)height {
    _isApplyingTitlebarHeight = YES;
    _titleBarHeightConstraint = configure_titlebar_height(self, height, _titleBarHeightConstraint);
    _isApplyingTitlebarHeight = NO;
}

- (void)configureTitlebarHeight:(CGFloat)height {
    _configuredTitlebarHeight = height;
    [self applyTitlebarHeight:height];
    [self observeTitlebarContainerIfNeeded];
}

- (void)applyFullscreenTitlebarHeight {
    [self applyTitlebarHeight:DEFAULT_TITLEBAR_HEIGHT];
}

- (void)restoreConfiguredTitlebarHeight {
    if (_configuredTitlebarHeight > 0) {
        [self applyTitlebarHeight:_configuredTitlebarHeight];
    }
}

- (void)observeTitlebarContainerIfNeeded {
    if (_observingTitlebarContainer) return;
    NSView *containerView = get_titlebar_container_view(self);
    if (!containerView) return;
    [containerView setPostsFrameChangedNotifications:YES];
    [[NSNotificationCenter defaultCenter] addObserver:self
                                             selector:@selector(titlebarContainerFrameDidChange:)
                                                 name:NSViewFrameDidChangeNotification
                                               object:containerView];
    _observingTitlebarContainer = YES;
}

- (void)titlebarContainerFrameDidChange:(NSNotification *)notification {
    if (_isApplyingTitlebarHeight) return;
    if (_configuredTitlebarHeight <= 0) return;
    BOOL isFullscreen = (self.styleMask & NSWindowStyleMaskFullScreen) != 0;
    if (isFullscreen) return;
    // Defer to avoid modifying constraints in the middle of an active layout pass.
    dispatch_async(dispatch_get_main_queue(), ^{
      [self applyTitlebarHeight:_configuredTitlebarHeight];
    });
}

- (void)setSuppressFrameConstraintsDuringDrag:(BOOL)value {
    _suppressFrameConstraintsDuringDrag = value;
}

- (BOOL)canBecomeMainWindow {
    return YES;
}

- (BOOL)canBecomeKeyWindow {
    return YES;
}

- (NSRect)constrainFrameRect:(NSRect)frameRect toScreen:(NSScreen *)screen {
    if (_suppressFrameConstraintsDuringDrag) {
        return frameRect;
    }
    return [super constrainFrameRect:frameRect toScreen:screen];
}

- (void)sendEvent:(NSEvent *)event {
    switch (event.type) {
        // In some cases, NSWindow's default sendEvent: implementation will dispatch a MouseDown
        // event and subsequent MouseDragged events to the content view, but then dispatch the
        // remaining MouseDragged events and MouseUp event elsewhere.
        // This is inconsistent with the Cocoa event architecture documentation
        // (https://developer.apple.com/library/archive/documentation/Cocoa/Conceptual/EventOverview/EventArchitecture/EventArchitecture.html),
        // but it's unclear how or why the events get redirected.
        // This breaks drag-and-drop for panes and tabs (see CLD-2581), so we work around it with
        // custom dispatching.
        case NSEventTypeLeftMouseUp:
            [self.contentView mouseUp:event];
            break;
        case NSEventTypeLeftMouseDragged:
            [self.contentView mouseDragged:event];
            break;

        // The NSWindow's default sendEvent: implementation does not propagate RightMouseDown events
        // from the application title bar to the content view when running a development build
        // locally, though it is unclear why. This breaks the right-click context menu for tabs on
        // local builds, so we propagate the RightMouseDown event manually.
        case NSEventTypeRightMouseDown:
            [self.contentView rightMouseDown:event];
            break;
        default:
            [super sendEvent:event];
            break;
    }
}

- (void)dealloc {
    [[NSNotificationCenter defaultCenter] removeObserver:self];
    warp_dealloc_window(self);
    [super dealloc];
}

- (void)setNeedsDisplayAsync {
    NSView *contentView = [self contentView];
    dispatch_async(dispatch_get_main_queue(), ^{
      [contentView setNeedsDisplay:YES];
    });
}

- (BOOL)performKeyEquivalent:(NSEvent *)event {
    // We need to bypass the default performKeyEquivalent implementation which, in the case of
    // having keybinding conflicts with MacOS itself, yields priority to the OS.
    if ([event type] == NSEventTypeKeyDown) {
        // Skip the key-equivalent priority path while the IME has marked text. Arrow keys carry
        // NSEventModifierFlagFunction, so AppKit delivers them here before keyDown:. If we call
        // keyDownImpl and Rust suppresses the keystroke (composing mode), we return NO, and AppKit
        // proceeds to call keyDown: — running interpretKeyEvents a second time for the same event.
        // See #9709.
        if ([(WarpHostView *)self.contentView hasMarkedText]) {
            return [super performKeyEquivalent:event];
        }

        NSApplication *application = [NSApplication sharedApplication];

        // If we are recording a keystroke for an EditableBinding.
        BOOL keyBindingsDisabled = warp_app_are_key_bindings_disabled_for_window(application, self);
        // If Warp has assigned a binding for this keystroke.
        BOOL keystrokeIsAssigned = warp_app_has_binding_for_keystroke(application, event);

        BOOL triggersCustomAction = warp_app_has_custom_action_for_keystroke(application, event);

        if (keyBindingsDisabled || (keystrokeIsAssigned && !triggersCustomAction)) {
            if ([self.contentView keyDownImpl:event]) {
                return YES;
            }
        }
    }

    return [super performKeyEquivalent:event];
}

- (void)closeWindowAsync:(BOOL)forceTermination {
    dispatch_async(dispatch_get_main_queue(), ^{
      WarpWindowDelegate *delegate = self.delegate;
      if (forceTermination) {
          [delegate setForceTermination];
          // Bypass performClose: (which can be deferred or vetoed by the
          // delegate's shouldClose) and tear the window down right away.
          [self close];
      } else {
          [self performClose:nil];
      }
    });
}

- (void)makeKeyAndOrderFront:(id)sender {
    if ([self testMode]) {
        // To avoid any issues due to the behavior of the developer using their
        // machine and modifying the global window stack, we instead hide the
        // window entirely, and track z-positioning in our own window position
        // stack.
        [self orderOut:sender];
        [windowOrderForTests addObject:@(self.windowNumber)];
    } else {
        [super makeKeyAndOrderFront:sender];
    }
}

- (void)zoomAsync:(id)sender {
    dispatch_async(dispatch_get_main_queue(), ^{
      [self zoom:sender];
    });
}

- (void)orderOut:(id)sender {
    if ([self testMode]) {
        [windowOrderForTests removeObject:@(self.windowNumber)];
    }

    [super orderOut:sender];
}

// Note this returns a retained object ("create" rule).
+ (WarpWindow *)createWithContentRect:(NSRect)contentRect
                          metalDevice:(id)metalDevice
                       hidingTitleBar:(BOOL)hideTitleBar
           backgroundBlurRadiusPixels:(uint8)backgoundBlurRadiusPixels
                         withTestMode:(BOOL)testMode {
    NSWindowStyleMask mask = warpWindowMask;

    if (hideTitleBar) {
        mask |= NSWindowStyleMaskFullSizeContentView;
    }

    WarpWindow *window_result = [[WarpWindow alloc] initWithContentRect:contentRect
                                                              styleMask:mask
                                                                backing:NSBackingStoreBuffered
                                                                  defer:NO];
    init_warp_nswindow(window_result, testMode, hideTitleBar);

    return window_result;
}

@end

// A panel is basically a NSWindow with the exception that it could be displayed
// above fullscreen apps.
@interface WarpPanel : NSPanel <WarpWindowProtocol>
@end

@implementation WarpPanel {
    // The windowState is managed on the Rust side.
    void *windowState;
    // Height constraint for the titlebar view (also indicates if constraints are configured)
    NSLayoutConstraint *_titleBarHeightConstraint;
    // The last height set via configureTitlebarHeight: (i.e. from Rust).
    CGFloat _configuredTitlebarHeight;
    // Whether we have registered for titlebar container frame change notifications.
    BOOL _observingTitlebarContainer;
    // Guard to prevent re-entrancy when we change the container frame ourselves.
    BOOL _isApplyingTitlebarHeight;
}

@synthesize testMode;
@synthesize hideTitleBar;

- (void)applyTitlebarHeight:(CGFloat)height {
    _isApplyingTitlebarHeight = YES;
    _titleBarHeightConstraint = configure_titlebar_height(self, height, _titleBarHeightConstraint);
    _isApplyingTitlebarHeight = NO;
}

- (void)configureTitlebarHeight:(CGFloat)height {
    _configuredTitlebarHeight = height;
    [self applyTitlebarHeight:height];
    [self observeTitlebarContainerIfNeeded];
}

- (void)applyFullscreenTitlebarHeight {
    [self applyTitlebarHeight:DEFAULT_TITLEBAR_HEIGHT];
}

- (void)restoreConfiguredTitlebarHeight {
    if (_configuredTitlebarHeight > 0) {
        [self applyTitlebarHeight:_configuredTitlebarHeight];
    }
}

- (void)observeTitlebarContainerIfNeeded {
    if (_observingTitlebarContainer) return;
    NSView *containerView = get_titlebar_container_view(self);
    if (!containerView) return;
    [containerView setPostsFrameChangedNotifications:YES];
    [[NSNotificationCenter defaultCenter] addObserver:self
                                             selector:@selector(titlebarContainerFrameDidChange:)
                                                 name:NSViewFrameDidChangeNotification
                                               object:containerView];
    _observingTitlebarContainer = YES;
}

- (void)titlebarContainerFrameDidChange:(NSNotification *)notification {
    if (_isApplyingTitlebarHeight) return;
    if (_configuredTitlebarHeight <= 0) return;
    BOOL isFullscreen = (self.styleMask & NSWindowStyleMaskFullScreen) != 0;
    if (isFullscreen) return;
    // Defer to avoid modifying constraints in the middle of an active layout pass.
    dispatch_async(dispatch_get_main_queue(), ^{
      [self applyTitlebarHeight:_configuredTitlebarHeight];
    });
}

- (BOOL)canBecomeMainWindow {
    return YES;
}

- (BOOL)canBecomeKeyWindow {
    return YES;
}

- (BOOL)isExcludedFromWindowsMenu {
    return NO;
}

- (void)dealloc {
    [[NSNotificationCenter defaultCenter] removeObserver:self];
    warp_dealloc_window(self);
    [super dealloc];
}

- (void)setNeedsDisplayAsync {
    NSView *contentView = [self contentView];
    dispatch_async(dispatch_get_main_queue(), ^{
      [contentView setNeedsDisplay:YES];
    });
}

- (void)closeWindowAsync:(BOOL)forceTermination {
    dispatch_async(dispatch_get_main_queue(), ^{
      WarpWindowDelegate *delegate = self.delegate;
      [delegate setForceTermination];
      [self close];
    });
}

- (void)performClose:(id)sender {
    warp_dispatch_standard_action(self, [sender tag]);
}

- (void)makeKeyAndOrderFront:(id)sender {
    if ([self testMode]) {
        // To avoid any issues due to the behavior of the developer using their
        // machine and modifying the global window stack, we instead hide the
        // window entirely, and track z-positioning in our own window position
        // stack.
        [self orderOut:sender];
        [windowOrderForTests addObject:@(self.windowNumber)];
    } else {
        [super makeKeyAndOrderFront:sender];
    }
}

- (void)orderOut:(id)sender {
    if ([self testMode]) {
        [windowOrderForTests removeObject:@(self.windowNumber)];
    }

    [super orderOut:sender];
}

- (void)positionPinnedPanel {
    previouslyActiveAppPID = [PreviousStateHelper storePreviousState];

    // NSFloatingWindowLevel allows us to float above all other normal application
    // windows but also not overlap with user's dock, menu bar, spotlight and Raycast.
    self.level = NSFloatingWindowLevel;

    // These collectionBehavior makes sure the panel could join fullscreen space.
    self.collectionBehavior =
        (self.collectionBehavior | NSWindowCollectionBehaviorCanJoinAllSpaces |
         NSWindowCollectionBehaviorFullScreenAuxiliary);

    [self setMovable:NO];
    [[NSApplication sharedApplication] activateIgnoringOtherApps:YES];
    [self makeKeyAndOrderFront:nil];
}

// Note this returns a retained object ("create" rule).
+ (WarpPanel *)createWithContentRect:(NSRect)contentRect
                         metalDevice:(id)metalDevice
                      hidingTitleBar:(BOOL)hideTitleBar
          backgroundBlurRadiusPixels:(uint8)backgoundBlurRadiusPixels
                        withTestMode:(BOOL)testMode {
    NSWindowStyleMask mask = warpWindowMask | NSWindowStyleMaskNonactivatingPanel;

    if (hideTitleBar) {
        mask |= NSWindowStyleMaskFullSizeContentView;
    }

    WarpPanel *window_result = [[WarpPanel alloc] initWithContentRect:contentRect
                                                            styleMask:mask
                                                              backing:NSBackingStoreBuffered
                                                                defer:NO];
    init_warp_nswindow(window_result, testMode, hideTitleBar);

    return window_result;
}

@end

void set_window_background_blur_radius(id window, uint8 blurRadiusPixels) {
    int windowNumber = [window windowNumber];
    CGSConnectionID con = CGSDefaultConnectionForThread();
    if (con) {
        CGSSetWindowBackgroundBlurRadiusFunction *function =
            GetCGSSetWindowBackgroundBlurRadiusFunction();
        if (function) {
            function(con, windowNumber, (int)MAX(1, blurRadiusPixels));
        }
    }
}

// Attaches a WarpWindowDelegate to |window| and ties its lifetime to the window.
//
// NSWindow.delegate is a weak property, so the delegate must be kept alive
// externally. We do this by associating it with the window via
// objc_setAssociatedObject, which retains the delegate and releases it when
// the window is deallocated. The caller's +1 from alloc/init is then balanced
// by the final [delegate release].
static void attach_warp_window_delegate(NSWindow *window) {
    WarpWindowDelegate *delegate = [[WarpWindowDelegate alloc] init];
    [window setDelegate:delegate];
    objc_setAssociatedObject(window, kWarpWindowDelegateAssocKey, delegate,
                             OBJC_ASSOCIATION_RETAIN_NONATOMIC);
    [delegate release];
}

// \return a new, retained WarpPanel with the given content rect.
id create_warp_nspanel(NSRect contentRect, id metalDevice, BOOL hideTitleBar,
                       uint8 backgroundBlurRadiusPixels, BOOL testMode) {
    NSAutoreleasePool *pool = [[NSAutoreleasePool alloc] init];

    if (testMode) {
        dispatch_once(&windowOrderOnce, ^{
          windowOrderForTests = [[NSMutableArray alloc] init];
        });
    }

    WarpPanel *window = [WarpPanel createWithContentRect:contentRect
                                             metalDevice:metalDevice
                                          hidingTitleBar:hideTitleBar
                              backgroundBlurRadiusPixels:backgroundBlurRadiusPixels
                                            withTestMode:testMode];

    WarpHostView *hostView = [[[WarpHostView alloc] initWithFrame:contentRect
                                                      metalDevice:metalDevice
                                               enableTitlebarDrag:NO
                                                         testMode:testMode] autorelease];

    attach_warp_window_delegate(window);

    window.contentView = hostView;
    [window makeFirstResponder:hostView];
    set_window_background_blur_radius(window, backgroundBlurRadiusPixels);
    [pool release];
    return window;
}

// \return a new, retained WarpWindow with the given content rect.
id create_warp_nswindow(NSRect contentRect, id metalDevice, BOOL hideTitleBar,
                        uint8 backgroundBlurRadiusPixels, BOOL testMode) {
    NSAutoreleasePool *pool = [[NSAutoreleasePool alloc] init];

    if (testMode) {
        dispatch_once(&windowOrderOnce, ^{
          windowOrderForTests = [[NSMutableArray alloc] init];
        });
    }

    WarpWindow *window = [WarpWindow createWithContentRect:contentRect
                                               metalDevice:metalDevice
                                            hidingTitleBar:hideTitleBar
                                backgroundBlurRadiusPixels:backgroundBlurRadiusPixels
                                              withTestMode:testMode];

    WarpHostView *hostView = [[[WarpHostView alloc] initWithFrame:contentRect
                                                      metalDevice:metalDevice
                                               enableTitlebarDrag:YES
                                                         testMode:testMode] autorelease];

    attach_warp_window_delegate(window);

    window.contentView = hostView;
    [window makeFirstResponder:hostView];
    set_window_background_blur_radius(window, backgroundBlurRadiusPixels);
    [pool release];
    return window;
}

BOOL is_warp_window(id window) {
    return [window isKindOfClass:[WarpWindow class]] || [window isKindOfClass:[WarpPanel class]];
}

// Returns the front-most window in the app's window list, or null if there are
// no open windows.
NSWindow *get_frontmost_window() {
    NSApplication *app = [NSApplication sharedApplication];

    if (windowOrderForTests != NULL) {
        if ([windowOrderForTests count] == 0) {
            return NULL;
        }
        return [app windowWithWindowNumber:[[windowOrderForTests lastObject] intValue]];
    }

    __block NSWindow *frontmost_window = NULL;
    [app enumerateWindowsWithOptions:NSWindowListOrderedFrontToBack
                          usingBlock:^(NSWindow *window, BOOL *stop) {
                            frontmost_window = window;
                            *stop = YES;
                          }];
    return frontmost_window;
}

// |sends accessibility notification and sets appropriate a11y-related fields.
// @param window - id of the window for which the a11y content is set
// @param value - the value of the hovered field
// @param help - helper text (the difference between this and value is mostly in semantics)
// @param warpRole - the role of the given element (we're using our own, internally defined roles,
//                    check warpui::accessibility)
// @param setFrame - boolean value that determines whether the passed frame should be set
// @param frame - rectangle that describes where the actual highlighted element is on the screen
void set_accessibility_contents(id window, NSString *value, NSString *help, NSString *warpRole,
                                BOOL setFrame, NSRect frame) {
    // Setting the standard parameters used for indicating accessibility features
    [window setAccessibilityLabel:help];
    [window setAccessibilityValue:value];
    // "use" the role variable temporarily until we re-introduce its usage.
    (void)warpRole;
    [window setAccessibilityValueDescription:value];
    if (setFrame) {
        [window setAccessibilityFrame:frame];
    }

    [window setAccessibilityElement:YES];
    [window setAccessibilityFocused:YES];

    // Sending an Accessibility notification with highest priority, effecivaly making our content
    // be most important and read first.
    id objects[] = {[NSString stringWithFormat:@"%@ %@", value, help], @"90" /* high priority */};
    id keys[] = {NSAccessibilityAnnouncementKey, NSAccessibilityPriorityKey};
    NSUInteger count = sizeof(objects) / sizeof(id);
    NSDictionary *userInfo = [NSDictionary dictionaryWithObjects:objects forKeys:keys count:count];
    NSAccessibilityPostNotificationWithUserInfo(
        window, NSAccessibilityAnnouncementRequestedNotification, userInfo);
}

void set_window_bounds(id window, NSRect frame) { [window setFrame:frame display:YES]; }

void open_file_path(NSString *pathString) {
    NSString *path = [pathString stringByExpandingTildeInPath];
    NSURL *url = [[NSURL fileURLWithPath:path] standardizedURL];
    [[NSWorkspace sharedWorkspace] openURL:url];
}

void open_file_path_in_explorer(NSString *pathString) {
    NSString *path = [pathString stringByExpandingTildeInPath];
    NSURL *url = [[NSURL fileURLWithPath:path] standardizedURL];

    // Dispatch this asynchronously on the main thread to avoid double-borrow
    // errors; see https://warpdotdev.sentry.io/issues/4264975772.
    dispatch_async(dispatch_get_main_queue(), ^{
      [[NSWorkspace sharedWorkspace] activateFileViewerSelectingURLs:@[ url ]];
    });
}

void open_file_picker(void *callback, NSArray<NSString *> *fileTypes, BOOL allowFiles,
                      BOOL allowFolders, BOOL allowMultiSelection) {
    // Create an open panel.
    NSOpenPanel *openPanel = [NSOpenPanel openPanel];
    // Set restrictions on which types of files users can pick.
    [openPanel setAllowsMultipleSelection:allowMultiSelection];
    [openPanel setCanChooseDirectories:allowFolders];
    [openPanel setCanCreateDirectories:allowFolders];
    [openPanel setCanChooseFiles:allowFiles];

    if (@available(macOS 11, *)) {
        NSMutableArray *contentTypes = [NSMutableArray array];
        for (NSString *fileType in fileTypes) {
            if ([fileType isEqualToString:@"Image"]) {
                [contentTypes addObject:UTTypeImage];
            } else if ([fileType isEqualToString:@"Markdown"]) {
                UTType *markdownType = [UTType typeWithFilenameExtension:@"md"];
                [contentTypes addObject:markdownType];
            } else if ([fileType isEqualToString:@"Yaml"]) {
                [contentTypes addObject:UTTypeYAML];
            }
        }

        [openPanel setAllowedContentTypes:contentTypes];
    } else {
        NSMutableArray *contentTypes = [NSMutableArray array];
        for (NSString *fileType in fileTypes) {
            if ([fileType isEqualToString:@"Image"]) {
                [contentTypes addObjectsFromArray:[NSImage imageTypes]];
            } else if ([fileType isEqualToString:@"Markdown"]) {
                [contentTypes addObject:@"md"];
            } else if ([fileType isEqualToString:@"Yaml"]) {
                [contentTypes addObject:@"yaml"];
                [contentTypes addObject:@"yml"];
            }
        }

        [openPanel setAllowedFileTypes:contentTypes];
    }

    // Open panel as sheet on main window.
    [openPanel beginWithCompletionHandler:^(NSInteger result) {
      // warp_open_panel_file_selected must be called unconditionally to avoid a memory leak
      if (result == NSModalResponseOK) {
          dispatch_async(dispatch_get_main_queue(), ^{
            warp_open_panel_file_selected([openPanel URLs], callback);
          });
      } else {
          dispatch_async(dispatch_get_main_queue(), ^{
            warp_open_panel_file_selected([NSArray array], callback);
          });
      }
    }];
}

void open_save_file_picker(void *callback, NSString *defaultFilename, NSString *defaultDirectory) {
    NSSavePanel *savePanel = [NSSavePanel savePanel];

    // Hide the NSSavePanel title bar entirely.
    [savePanel setTitlebarAppearsTransparent:YES];
    [savePanel setTitleVisibility:NSWindowTitleHidden];

    [savePanel setNameFieldStringValue:defaultFilename];

    if ([defaultDirectory length] > 0) {
        NSURL *directoryURL = [NSURL fileURLWithPath:defaultDirectory];
        [savePanel setDirectoryURL:directoryURL];
    }

    // Show save panel as sheet
    [savePanel beginWithCompletionHandler:^(NSInteger result) {
      // warp_save_panel_file_selected must be called unconditionally to avoid a memory leak
      if (result == NSModalResponseOK) {
          dispatch_async(dispatch_get_main_queue(), ^{
            warp_save_panel_file_selected([savePanel URL], callback);
          });
      } else {
          dispatch_async(dispatch_get_main_queue(), ^{
            warp_save_panel_file_selected(nil, callback);
          });
      }
    }];
}

// Open a given url.
void open_url(NSString *urlString) {
    NSURL *url = [NSURL URLWithString:urlString];
    [[NSWorkspace sharedWorkspace] openURL:url];
}

void hide_app() {
    NSApplication *app = [NSApplication sharedApplication];

    if (![app isHidden]) {
        [app hide:nil];
    }
}

void activate_app() {
    NSApplication *app = [NSApplication sharedApplication];

    if (![app isActive]) {
        [app activateIgnoringOtherApps:YES];
    }
}

void show_window_and_focus_app(WarpWindow<WarpWindowProtocol> *window, bool bringToFront) {
    previouslyActiveAppPID = [PreviousStateHelper storePreviousState];

    // Make sure the window is included in the application's window list.  This
    // is automatically done by the framework for normal windows, but we need to
    // do this explicitly for hotkey windows, as they subclass NSPanel (which
    // requires explicit registration in the window list).
    [NSApp addWindowsItem:window title:[window title] filename:NO];

    if (bringToFront) {
        [window makeKeyAndOrderFront:nil];
    } else {
        [window makeKeyWindow];
    }

    // There are some edge cases with the hot key window in a multi-screen setup that toggling
    // the hotkey will activate the app and only bring forward a normal window. This code makes
    // sure that we are bringing forward the hotkey window
    if (![[NSApplication sharedApplication] isActive]) {
        // Creates a static observer so it can be referenced in the observer callback.
        __block id observer;
        observer = [[NSNotificationCenter defaultCenter]
            addObserverForName:NSApplicationDidBecomeActiveNotification
                        object:nil
                         queue:NULL
                    usingBlock:^(NSNotification *note __unused) {
                      // Make key and order front again after the app has activated to make
                      // sure the toggled window is focused after initializing.
                      [window makeKeyAndOrderFront:nil];
                      [[NSNotificationCenter defaultCenter] removeObserver:observer];
                    }];

        [[NSApplication sharedApplication] activateIgnoringOtherApps:YES];
    }
}

void hide_window(WarpWindow<WarpWindowProtocol> *window) {
    NSRunningApplication *runningApp = [[NSWorkspace sharedWorkspace] frontmostApplication];

    // Don't activate to previous state if:
    // 1. user is explicitly switching app by clicking into another app or hitting cmd-tab.
    //    We only want to focus the previous app if the window was hidden while our app is active.
    // 2. The window was hidden because a modal popped up. We don't want to hide the modal.
    NSWindow *activeWindow = [[NSApplication sharedApplication] keyWindow];
    if ([runningApp.bundleIdentifier isEqualToString:[[NSBundle mainBundle] bundleIdentifier]] &&
        ![activeWindow isModalPanel]) {
        [PreviousStateHelper activatePreviousState:previouslyActiveAppPID];
    }
    previouslyActiveAppPID = nil;

    // Order out removes window from the screen but still maintains the NSWindow object.
    [window orderOut:nil];
}

// Sets the per-window opacity. Unlike `hide_window`, this does not change the
// window's z-order, key state, or the app's active state — making it a much
// cheaper way to visually hide a window (e.g. a tab drag preview) without
// triggering AppKit's `orderOut:` machinery or the previous-app activation
// dance.
void set_window_alpha(WarpWindow<WarpWindowProtocol> *window, double alpha) {
    [window setAlphaValue:alpha];
}

void set_window_title(id window, NSString *title) {
    if ([window isKindOfClass:[WarpPanel class]] && [window isVisible]) {
        // For the hotkey window (which is an NSPanel), we need to explicitly
        // add the panel to the windows list.  `changeWindowsItem` will add the
        // panel to the list if it isn't already there.
        [NSApp changeWindowsItem:window title:title filename:NO];
    }

    [window setTitle:title];
}

void set_titlebar_height(id window, CGFloat height) {
    if ([window conformsToProtocol:@protocol(WarpWindowProtocol)]) {
        [(id<WarpWindowProtocol>)window configureTitlebarHeight:height];
    }
}

void position_and_order_front(WarpWindow<WarpWindowProtocol> *window) {
    // Called from Rust to position ourselves and order front.
    // TODO: use NSUserDefaults to remember window locations.
    // We cascade relative to the front-most window.  This will typically be the
    // main/key window, but when the app is inactive, we want to cascade
    // relative to the top window in the application's stack.
    NSWindow *mainWindow = get_frontmost_window();
    if (!mainWindow) {
        // No window onscreen.
        [window center];
    } else {
        // Cascade relative to the main window.
        // The first cascade does not move the main window as the argument is 0.
        // The next cascade moves this window.
        NSPoint cascadePoint = [mainWindow cascadeTopLeftFromPoint:NSZeroPoint];
        [window cascadeTopLeftFromPoint:cascadePoint];
    }

    [window makeKeyAndOrderFront:nil];
}

void position_at_given_location(WarpWindow<WarpWindowProtocol> *window, NSPoint origin) {
    // Use an explicit top-left point for drag handoff windows. Unlike the cascade helper above,
    // tab transfer needs deterministic placement at a Rust-provided screen position.
    NSPoint topLeft = NSMakePoint(origin.x, origin.y + [window frame].size.height);
    [window setFrameTopLeftPoint:topLeft];
    [window makeKeyAndOrderFront:nil];
}

void order_front_without_focus(WarpWindow<WarpWindowProtocol> *window, NSPoint origin) {
    [window setFrameOrigin:origin];
    [window orderFront:nil];
}
