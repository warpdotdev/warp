#import <AppKit/AppKit.h>

// Enforces that multiple windows don't transition to fullscreen at
// the same time.
@interface FullscreenWindowManager : NSObject
// Queues a window to be transitioned to fullscreen. Not thread-safe.
- (void)enqueueWindow:(NSWindow *)window;
@end
