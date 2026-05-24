#import <AppKit/AppKit.h>

#import "app.h"

NSModalResponse configureAndRunModal(NSAlert *alert, NSApplication *app) {
    alert.showsSuppressionButton = YES;

    // It is generally frowned-upon to be overly assertive about putting our windows
    // in the user's face. However, it is reasonable to do this before showing our modal.
    // If we don't make ourselves the top active app, our modal might show up BEHIND an-
    // other app's window.
    [app activateIgnoringOtherApps:YES];
    NSModalResponse response = [alert runModal];

    return response;
}
