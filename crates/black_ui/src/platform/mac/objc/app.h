#import <AppKit/AppKit.h>
#import <Carbon/Carbon.h>
#import <UserNotifications/UserNotifications.h>

// Our NSApplication subclass.
@interface WarpApplication : NSApplication
@end

// WarpDelegate is the delegate of the NSApp and also all menus.
@interface WarpDelegate
    : NSObject <NSApplicationDelegate, NSMenuDelegate, UNUserNotificationCenterDelegate>

@property(strong) NSMenu *dockMenu;

@end

// Functions implemented in Rust.
void warp_app_will_finish_launching(id app);
void warp_app_did_become_active(id app);
void warp_app_did_resign_active(id app);
void warp_app_will_terminate(id app);
void warp_app_open_files(id app, id filenames);
void warp_app_send_global_keybinding(id app, NSUInteger modifiers, NSUInteger key_code);
void warp_app_new_window(id app);
void warp_app_window_did_resize(id app);
void warp_app_window_did_move(id app);
void warp_app_window_will_close(id app, id window);
void warp_app_screen_did_change(id app);
void cpu_awakened(id app);
void cpu_will_sleep(id app);
void warp_app_active_window_changed(id app);
void warp_app_notification_clicked(id app, double date, id data);
void warp_app_open_urls(id app, id urls);
void warp_app_os_appearance_changed(id app);
BOOL warp_app_should_terminate_app(id app);
BOOL warp_app_should_close_window(id app, id window);
BOOL warp_app_are_key_bindings_disabled_for_window(id app, id window);
BOOL warp_app_has_binding_for_keystroke(id app, id event);
BOOL warp_app_has_custom_action_for_keystroke(id app, id event);
void warp_app_disable_warning_modal(id app);
void warp_app_internet_reachability_changed(id app, BOOL can_reach);
void warp_app_process_modal_response(id app, NSUInteger modal_id, NSModalResponse response,
                                     BOOL disable_modal);
