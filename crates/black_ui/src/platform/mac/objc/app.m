#import <AppKit/AppKit.h>
#import <Carbon/Carbon.h>
#import <ServiceManagement/ServiceManagement.h>
#import <UserNotifications/UserNotifications.h>

#import "alert.h"
#import "app.h"
#import "host_view.h"
#import "hotkey.h"
#import "menus.h"

#import "reachability.h"

static void *NSAppThemeChangeContext = &NSAppThemeChangeContext;

NSMutableDictionary<NSNumber *, WarpHotKey *> *_hotKeys;
UInt32 _nextHotKeyID;

OSStatus HotkeyPressedHandler(EventHandlerCallRef _inCaller __unused, EventRef inEvent,
                              void *inUserData);
OSStatus HotkeyPressedHandler(EventHandlerCallRef _inCaller __unused, EventRef inEvent,
                              void *inUserData) {
    EventHotKeyID hotKeyID;

    // Get the hotKeyID corresponding to the pressed hot key.
    if (GetEventParameter(inEvent, kEventParamDirectObject, typeEventHotKeyID, nil,
                          sizeof(EventHotKeyID), nil, &hotKeyID)) {
        return eventNotHandledErr;
    }

    WarpHotKey *hotkey = _hotKeys[@(hotKeyID.id)];
    if (hotkey) {
        warp_app_send_global_keybinding((NSApplication *)inUserData, hotkey->_modifierKeys,
                                        hotkey->_keyCode);
        return noErr;
    }

    return eventNotHandledErr;
}

BOOL isDarkMode() {
    NSAppearanceName name = [NSApp.effectiveAppearance
        bestMatchFromAppearancesWithNames:@[ NSAppearanceNameAqua, NSAppearanceNameDarkAqua ]];
    return name == NSAppearanceNameDarkAqua;
}

NSArray *getFilePathsFromPasteboard() {
    NSPasteboard *pb = [NSPasteboard generalPasteboard];
    NSArray *types = [pb types];

    if ([types containsObject:NSPasteboardTypeFileURL]) {
        return [pb getFilePaths];
    }

    return [NSArray array];
}

void *registerGlobalHotkey(NSUInteger key, NSUInteger modifiers) {
    EventHotKeyRef hotKeyRef = NULL;
    EventHotKeyID hotKeyID = {0, _nextHotKeyID};
    if (RegisterEventHotKey((UInt32)key, (UInt32)modifiers, hotKeyID, GetEventDispatcherTarget(), 0,
                            &hotKeyRef)) {
        return nil;
    };
    [_hotKeys setObject:[[[WarpHotKey alloc] initWithEventHotKey:hotKeyRef
                                                         keyCode:key
                                                    modifierKeys:modifiers] autorelease]
                 forKey:@(hotKeyID.id)];
    _nextHotKeyID++;
    return nil;
}

void *unregisterGlobalHotkey(NSUInteger key, NSUInteger modifiers) {
    NSNumber *keyIdx;
    BOOL found = NO;

    for (NSNumber *hotKeyID in _hotKeys) {
        if ([[_hotKeys objectForKey:hotKeyID] hotKeyKeyAndModifierEquals:key
                                                            modifierKeys:modifiers]) {
            keyIdx = hotKeyID;
            found = YES;
            break;
        }
    }

    if (found) {
        UnregisterEventHotKey([_hotKeys objectForKey:keyIdx]->_eventHotKey);
        [_hotKeys removeObjectForKey:keyIdx];
    }
    return nil;
}

NSRect screenFrame() { return [[NSScreen mainScreen] frame]; }

NSUInteger activeScreenId() {
    return [[[[NSScreen mainScreen] deviceDescription] objectForKey:@"NSScreenNumber"]
        unsignedIntegerValue];
}

@interface WarpMenuItemDelegate : NSObject <NSMenuDelegate> {
    // Rust expects an ivar with this name.
    void *rustWrapper;
}
@end

@implementation WarpDelegate {
    // Rust expects an ivar with this name.
    void *rustWrapper;

    // Whether we have a pending active window change notification.
    BOOL hasPendingActiveWindowChange;

    // Internet reachability.
    Reachability *internetReachable;

    // Track the current reachable state so we don't double fire reachability state
    // changed events.
    NSNumber *isReachable;

    // Whether we should force termination.
    BOOL forceTermination;

    // Whether we should terminate the application upon the app
    // being hidden.  This allows us to hide the app before running any
    // slower termination logic.
    BOOL terminateOnHide;
}

- (id)init {
    [super init];
    NSNotificationCenter *defaultCenter = [NSNotificationCenter defaultCenter];
    [defaultCenter addObserver:self
                      selector:@selector(keyWindowChanged:)
                          name:NSWindowDidBecomeKeyNotification
                        object:nil];
    [defaultCenter addObserver:self
                      selector:@selector(keyWindowChanged:)
                          name:NSWindowDidResignKeyNotification
                        object:nil];

    [defaultCenter addObserver:self
                      selector:@selector(windowMoved:)
                          name:NSWindowDidMoveNotification
                        object:nil];
    [defaultCenter addObserver:self
                      selector:@selector(windowResized:)
                          name:NSWindowDidResizeNotification
                        object:nil];
    [defaultCenter addObserver:self
                      selector:@selector(screenChanged:)
                          name:NSApplicationDidChangeScreenParametersNotification
                        object:nil];

    // For the following notifications, we need to register them on the workspace
    // notification center, which is different from the default NSNotificationCenter.
    // See here for more details: https://developer.apple.com/library/archive/qa/qa1340/_index.html.
    NSNotificationCenter *workspaceCenter = [[NSWorkspace sharedWorkspace] notificationCenter];
    [workspaceCenter addObserver:self
                        selector:@selector(cpuAwakened:)
                            name:NSWorkspaceDidWakeNotification
                          object:nil];
    [workspaceCenter addObserver:self
                        selector:@selector(cpuWillSleep:)
                            name:NSWorkspaceWillSleepNotification
                          object:nil];

    // Tell the shared notification center to use the current view as the
    // `UNUserNotificationCenterDelegate` delegate. We only do this if the application
    // is bundled, otherwise the app will crash when trying to set the delegate. This allows
    // warpui to still be run via `cargo run` since the app is not bundled in this case. Note this
    // has no functional change in the non-bundled case since the app must be bundled for
    // notifications to actually be sent/received.
    NSString *bundleIdentifier = [[NSBundle mainBundle] bundleIdentifier];
    if (bundleIdentifier != nil && ![bundleIdentifier isEqualToString:(@"")]) {
        UNUserNotificationCenter *user_notification_center =
            [UNUserNotificationCenter currentNotificationCenter];
        user_notification_center.delegate = self;

        // Create and register the notification category.
        UNNotificationCategory *CustomizedNotification = [UNNotificationCategory
            categoryWithIdentifier:@"CUSTOMIZED_NOTIFICATION"
                           actions:@[]
                 intentIdentifiers:@[]
                           options:UNNotificationCategoryOptionCustomDismissAction];

        [user_notification_center
            setNotificationCategories:[NSSet setWithObjects:CustomizedNotification, nil]];
    }

    // Initiate the global hotkey handlers first so we could register them at the rust
    // side callback.
    EventTypeSpec eventType = {kEventClassKeyboard, kEventHotKeyPressed};
    InstallApplicationEventHandler(HotkeyPressedHandler, 1, &eventType, self, NULL);
    _hotKeys = [[NSMutableDictionary alloc] init];

    return self;
}

- (void)dealloc {
    [NSApp removeObserver:self forKeyPath:@"effectiveAppearance" context:NSAppThemeChangeContext];
    [[NSNotificationCenter defaultCenter] removeObserver:self];
    [internetReachable stopNotifier];
    [internetReachable release];
    [super dealloc];
}

- (void)applicationWillFinishLaunching:(NSNotification *)note {
    // On macOS 26, the autofill heuristic controller causes significant slowdowns.
    // It's not clear why, but other apps which use custom text inputs have reported
    // the same issue. See:
    // * Ghostty: https://github.com/ghostty-org/ghostty/pull/8625
    // * Zed: https://github.com/zed-industries/zed/issues/33182
    // * Twitter thread discussing the issue: https://x.com/mitchellh/status/1967324131801915875
    NSUserDefaults *defaults = [NSUserDefaults standardUserDefaults];
    [defaults setBool:NO forKey:@"NSAutoFillHeuristicControllerEnabled"];

    if (rustWrapper) warp_app_will_finish_launching(note.object);
}

- (void)applicationDidFinishLaunching:(NSNotification *)note {
    [NSApp addObserver:self
            forKeyPath:@"effectiveAppearance"
               options:(NSKeyValueObservingOptionNew | NSKeyValueObservingOptionOld)
               context:NSAppThemeChangeContext];
}

- (void)observeValueForKeyPath:(NSString *)keyPath
                      ofObject:(id)object
                        change:(NSDictionary *)change
                       context:(void *)context {
    if (context == NSAppThemeChangeContext) {
        if (rustWrapper) warp_app_os_appearance_changed(self);
    } else {
        // Any unrecognized context must belong to super
        [super observeValueForKeyPath:keyPath ofObject:object change:change context:context];
    }
}

- (void)applicationDidBecomeActive:(NSNotification *)note {
    if (rustWrapper) warp_app_did_become_active(note.object);
}

- (void)setForceTermination {
    forceTermination = YES;
}

// Unfullscreens any windows that are currently fullscreen.
- (void)unfullscreenAllWindows:(NSApplication *)application {
    for (NSWindow *window in [application windows]) {
        if ((window.styleMask & NSWindowStyleMaskFullScreen) == NSWindowStyleMaskFullScreen) {
            [window toggleFullScreen:nil];
            return;
        }
    }
}

- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)application {
    BOOL okToTerminate = YES;

    // If this is the second termination attempt after we've already hidden the app, we can go ahead
    // and terminate.
    if (terminateOnHide) {
        return NSTerminateNow;
    }

    if (!forceTermination) {
        // Make sure the rust app doesn't have any reasons to interrupt quit, e.g. needs to relaunch
        // for autoupdate, but launching the new process failed.
        okToTerminate = warp_app_should_terminate_app(application);
    }

    if (okToTerminate) {
        // We want to hide the application before we start the teardown
        // process, to ensure the user isn't affected by any slow teardown
        // steps.  The tricky part here is that a call to `[NSApp hide]` isn't
        // handled synchronously, it is processed on the event loop.
        //
        // To work around this, we enqueue the hide on the event loop and set
        // some state so we know to resume termination of the application when
        // the hide takes effect.  As a fallback, if we never get notified that
        // the application was hidden, we always resume termination after 5s.
        // We deliberately do _not_ return `NSTerminateLater` here because it enables a special mode
        // of the event loop that is meant specifically for handling modals. We also make sure to
        // first exit any fullscreen windows before we hide--`NSApplication#hide` is a NOOP if there
        // are any full screen windows.

        [self unfullscreenAllWindows:application];
        [application hide:nil];
        terminateOnHide = YES;

        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC),
                       dispatch_get_main_queue(), ^{
                         [application terminate:nil];
                       });
    }
    return NSTerminateCancel;
}

- (void)applicationDidHide:(NSNotification *)note {
    if (terminateOnHide) {
        NSApplication *app = note.object;
        [app terminate:nil];
    }
}

- (void)applicationDidResignActive:(NSNotification *)note {
    if (rustWrapper) warp_app_did_resign_active(note.object);
}

- (void)applicationWillTerminate:(NSNotification *)note {
    if (rustWrapper) warp_app_will_terminate(note.object);
}

- (void)application:(NSApplication *)sender openFiles:(NSArray<NSString *> *)filenames {
    if (rustWrapper) warp_app_open_files(sender, filenames);
}

- (void)application:(NSApplication *)application openURLs:(NSArray<NSURL *> *)urls {
    if (rustWrapper) warp_app_open_urls(application, urls);
}

// This is called when clicking on the app in the Dock or from Finder.
// If there's no visible windows, we will open one.
- (BOOL)applicationShouldHandleReopen:(NSApplication *)app hasVisibleWindows:(BOOL)flag {
    if (rustWrapper && !flag) {
        warp_app_new_window(app);
        return NO;  // do nothing
    }
    return YES;
}

- (void)keyWindowChanged:(NSNotification *)note {
    // We use an async dispatch here for two reasons:
    //  1. When the active window changes, this will be called twice (once for resign, once for
    //     activated). We can coalesce these calls.
    //  2. When a new window is created, warp will activate it; if we recursively call back into
    //     warp then we will cause the app to be mutably borrowed while already borrowed.
    if (!hasPendingActiveWindowChange) {
        hasPendingActiveWindowChange = YES;
        dispatch_async(dispatch_get_main_queue(), ^{
          self->hasPendingActiveWindowChange = NO;
          if (self->rustWrapper) warp_app_active_window_changed(self);
        });
    }
}

- (void)windowMoved:(NSNotification *)note {
    // We need to use async dispatch here because the event loop in appkit calls the
    // app notification first before calling the window notification. Since we are updating
    // the window properties within the window notification, we need to make sure this
    // callback gets triggered after the window notification. Thus using the async dispatch
    // here ensures we always save the most up-to-date value within the database.
    dispatch_async(dispatch_get_main_queue(), ^{
      if (self->rustWrapper) warp_app_window_did_move(self);
    });
}

- (void)windowResized:(NSNotification *)note {
    dispatch_async(dispatch_get_main_queue(), ^{
      if (self->rustWrapper) warp_app_window_did_resize(self);
    });
}

- (void)screenChanged:(NSNotification *)note {
    dispatch_async(dispatch_get_main_queue(), ^{
      if (self->rustWrapper) warp_app_screen_did_change(self);
    });
}

- (void)cpuAwakened:(NSNotification *)note {
    dispatch_async(dispatch_get_main_queue(), ^{
      if (self->rustWrapper) cpu_awakened(self);
    });
}

- (void)cpuWillSleep:(NSNotification *)note {
    dispatch_async(dispatch_get_main_queue(), ^{
      if (self->rustWrapper) cpu_will_sleep(self);
    });
}

- (void)menuNeedsUpdate:(NSMenu *)menu {
    // Trigger warp_menu_item_needs_update for every item with our class set as its represented
    // object.
    Class warpHandlerClass = [WarpCustomMenuItemHandler class];
    for (NSMenuItem *item in menu.itemArray) {
        id obj = item.representedObject;
        if ([obj isKindOfClass:warpHandlerClass]) {
            [obj itemNeedsUpdate:item];
        }
    }
}

- (void)setReachabilityListener {
    internetReachable = [[Reachability reachabilityWithHostname:@"0.0.0.0"] retain];

    // Internet is reachable.
    internetReachable.reachableBlock = ^(Reachability *reach __unused) {
      // Update the UI on the main thread.
      dispatch_async(dispatch_get_main_queue(), ^{
        if (self->isReachable == nil || [self->isReachable intValue] == 0) {
            self->isReachable = [NSNumber numberWithBool:YES];
            if (self->rustWrapper) warp_app_internet_reachability_changed(self, YES);
        }
      });
    };

    // Internet is not reachable.
    internetReachable.unreachableBlock = ^(Reachability *reach __unused) {
      // Update the UI on the main thread.
      dispatch_async(dispatch_get_main_queue(), ^{
        if (self->isReachable == nil || [self->isReachable intValue] > 0) {
            self->isReachable = [NSNumber numberWithBool:NO];
            if (self->rustWrapper) warp_app_internet_reachability_changed(self, NO);
        }
      });
    };

    // Dispatch an initial call to check internet reachability so app could get notified
    // of the reachability status it starts in.
    dispatch_async(dispatch_get_global_queue(DISPATCH_QUEUE_PRIORITY_DEFAULT, 0), ^(void) {
      BOOL internetIsReachable = [internetReachable isReachable];
      dispatch_async(dispatch_get_main_queue(), ^{
        if (self->isReachable == nil) {
            self->isReachable = [NSNumber numberWithBool:internetIsReachable];
            if (self->rustWrapper)
                warp_app_internet_reachability_changed(self, internetIsReachable);
        }
      });
    });

    [internetReachable startNotifier];
}

// Returns a new NSMenu in the mac dock. Gets called every time we pull up the dock menu
- (NSMenu *)applicationDockMenu:(NSApplication *)sender {
    return self.dockMenu;
}

- (void)userNotificationCenter:(UNUserNotificationCenter *)center
    didReceiveNotificationResponse:(UNNotificationResponse *)response
             withCompletionHandler:(void (^)(void))completionHandler {
    // Handle what happens when the user clicks the notification. Warp doesn't support any actions
    // other than the default action currently.
    if ([response.actionIdentifier isEqualToString:UNNotificationDefaultActionIdentifier]) {
        NSDictionary *userInfo = response.notification.request.content.userInfo;
        NSString *data = userInfo[@"DATA"];

        if (rustWrapper) {
            warp_app_notification_clicked(self, response.notification.date.timeIntervalSince1970,
                                          data);
        }
    }
}

@end

@implementation WarpApplication {
    // Rust expects an ivar with this name.
    void *rustWrapper;
}

- (void)setForceTermination {
    WarpDelegate *delegate = (WarpDelegate *)self.delegate;
    [delegate setForceTermination];
}

- (void)showModal:(NSAlert *)alert modalId:(NSUInteger)modalId {
    dispatch_async(dispatch_get_main_queue(), ^{
      NSModalResponse response = configureAndRunModal(alert, self);

      BOOL disable_modal = alert.suppressionButton.state == NSControlStateValueOn;
      // Subtracting `NSAlertFirstButtonReturn` from `response` yields the 0-based index of the
      // button that was actually clicked.
      warp_app_process_modal_response(self, modalId, response - NSAlertFirstButtonReturn,
                                      disable_modal);
    });
}

@end

WarpApplication *get_warp_app() {
    // Set up the delegate (once).
    // The delegate is deliberately leaked.
    WarpApplication *app = [WarpApplication sharedApplication];
    static dispatch_once_t once;
    static id sharedDelegate;
    dispatch_once(&once, ^{
      sharedDelegate = [[WarpDelegate alloc] init];
      [app setDelegate:sharedDelegate];

      // Hack to work around the fact that warp is frequently tested as a
      // standalone (unbundled) binary.
      app.activationPolicy = NSApplicationActivationPolicyRegular;
    });
    return app;
}

// \return an empty NSMenu with the given title, setting up the delegate appropriately.
// The result is autoreleased.
NSMenu *make_delegated_menu(NSString *title) {
    NSMenu *result = [[[NSMenu alloc] initWithTitle:title] autorelease];
    result.delegate = (WarpDelegate *)[[WarpApplication sharedApplication] delegate];
    return result;
}

// Create Services, a system-defined standard menu on macOS
// The result is autoreleased.
NSMenuItem *make_services_menu_item() {
    // Create the services menu. `servicesMenu` retains, so autorelease our +1 ownership.
    NSApp.servicesMenu = [[[NSMenu alloc] init] autorelease];

    // Create menu item for it
    NSMenuItem *servicesItem = [[[NSMenuItem alloc] init] autorelease];
    servicesItem.title = @"Services";
    servicesItem.submenu = NSApp.servicesMenu;

    return servicesItem;
}

// \return a new menu item that wraps the given context pointer.
// The pointer will be provided back to Warp in the callbacks (see menus.h).
// The result is autoreleased.
NSMenuItem *make_warp_custom_menu_item(void *context) {
    WarpCustomMenuItemHandler *handler =
        [[[WarpCustomMenuItemHandler alloc] initWithContext:context] autorelease];

    // Sets action to NULL if menu item has submenu, so the menu doesn't close when item is clicked
    NSMenuItem *item = [[[NSMenuItem alloc] initWithTitle:@""
                                                   action:@selector(itemWasTriggered:)
                                            keyEquivalent:@""] autorelease];
    item.representedObject = handler;
    item.target = handler;
    return item;
}

NSString *executableInApplicationBundleWithIdentifier(NSString *bundle_path) {
    NSBundle *bundle = [NSBundle bundleWithPath:bundle_path];
    NSString *executable = [bundle.bundlePath stringByAppendingPathComponent:@"Contents/MacOS"];
    executable = [executable
        stringByAppendingPathComponent:[bundle
                                           objectForInfoDictionaryKey:(id)kCFBundleExecutableKey]];
    return executable;
}

NSString *absolutePathForApplicationBundleWithIdentifier(NSString *bundle_identifier) {
    NSURL *url =
        [[NSWorkspace sharedWorkspace] URLForApplicationWithBundleIdentifier:bundle_identifier];
    return url.path;
}

BOOL isVoiceOverEnabled() { return [[NSWorkspace sharedWorkspace] isVoiceOverEnabled]; }
