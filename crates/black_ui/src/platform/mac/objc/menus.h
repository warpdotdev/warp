#import <AppKit/AppKit.h>

// WarpCustomMenuItemHandler is set as both the target and represented object of NSMenuItem.
// It gives the Rust side a chance to dynamically update menu items, and
// respond to their actions.
@interface WarpCustomMenuItemHandler : NSObject <NSMenuItemValidation> {
    void *rustContext;
}

// Init, wrapping a pointer which is significant to Rust.
- (id)initWithContext:(void *)wrapper;

// Action set on menu items.
- (void)itemWasTriggered:(NSMenuItem *)item;

// Called when the menu item needs updating.
- (void)itemNeedsUpdate:(NSMenuItem *)item;

@end
