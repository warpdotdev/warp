#import <AppKit/AppKit.h>
#import <Carbon/Carbon.h>

@interface WarpHotKey : NSObject {
   @public
    EventHotKeyRef _eventHotKey;
   @public
    NSUInteger _keyCode;
   @public
    NSUInteger _modifierKeys;
}

- (instancetype)initWithEventHotKey:(EventHotKeyRef)eventHotKey
                            keyCode:(NSUInteger)keyCode
                       modifierKeys:(NSUInteger)modifierKeys;

- (BOOL)hotKeyKeyAndModifierEquals:(NSUInteger)keyCode modifierKeys:(NSUInteger)modifierKeys;

@end
