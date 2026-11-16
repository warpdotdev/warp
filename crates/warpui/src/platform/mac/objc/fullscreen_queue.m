#import "fullscreen_queue.h"
#import <AppKit/AppKit.h>

@implementation FullscreenWindowManager {
    // A LIFO queue of windows that want to transition to fullscreen.
    NSMutableArray<NSWindow *> *fullscreenQueue;

    // Whether or not there is currently a window transitioning to fullscreen. Note
    // that the absence of a locking mechanism makes the FullscreenWindowManager not
    // thread-safe.
    BOOL activeTransition;
}

- (instancetype)init {
    self = [super init];
    if (self) {
        fullscreenQueue = [[NSMutableArray alloc] init];
        activeTransition = NO;
    }

    [[NSNotificationCenter defaultCenter] addObserver:self
                                             selector:@selector(windowWillTransitionToFullscreen:)
                                                 name:NSWindowWillEnterFullScreenNotification
                                               object:nil];
    [[NSNotificationCenter defaultCenter] addObserver:self
                                             selector:@selector(windowDidTransitionToFullscreen:)
                                                 name:NSWindowDidEnterFullScreenNotification
                                               object:nil];
    return self;
}

- (void)enqueueWindow:(NSWindow *)window {
    [fullscreenQueue addObject:window];
    [self transitionNextWindowInQueue];
}

- (void)transitionNextWindowInQueue {
    if (activeTransition == YES) {
        return;
    }

    if ([fullscreenQueue count] > 0) {
        NSWindow *window = fullscreenQueue.firstObject;
        [fullscreenQueue removeObjectAtIndex:0];

        [window performSelector:@selector(toggleFullScreen:)];
    }
}

// Callback for when a window starts a fullscreen transition.
- (void)windowWillTransitionToFullscreen:(NSNotification *)notification {
    activeTransition = YES;
}

// Callback for when a window ends a fullscreen transition.
- (void)windowDidTransitionToFullscreen:(NSNotification *)notification {
    activeTransition = NO;
    [self transitionNextWindowInQueue];
}
@end
