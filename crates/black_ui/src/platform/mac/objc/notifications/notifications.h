#import <AppKit/AppKit.h>

// Requests authorization for notifications.
void requestNotificationPermissions(void* callback);

// This method, implemented in Rust, invokes the callback to allow the App to
// take action when the user has responded to the permissions request.
void warp_on_request_notification_permissions_completed(NSUInteger outcome_type, id outcome_msg,
                                                        void* callback);

// Sends a desktop notification.
void sendNotification(id, id, id, void*, BOOL);

// This method, implemented in Rust, invokes the callback to allow the App to
// take action when a notification fails to send.
void warp_on_notification_send_error(NSUInteger error_type, id error_msg, void* callback);
