#import "notifications.h"
#import "../app.h"

#import <UserNotifications/UserNotifications.h>

void requestNotificationPermissionsWithCompletionHandler(
    void (^completion_handler)(NSUInteger outcome_type, id outcome_msg)) {
    UNUserNotificationCenter *center = [UNUserNotificationCenter currentNotificationCenter];

    [center
        requestAuthorizationWithOptions:(UNAuthorizationOptionAlert + UNAuthorizationOptionSound +
                                         UNAuthorizationOptionBadge)
                      completionHandler:^(BOOL granted, NSError *_Nullable error) {
                        if (!granted) {
                            completion_handler(1, @"User denied request to receive notifications.");
                        } else if (error != nil) {
                            completion_handler(2, error.localizedDescription);
                        } else {
                            // Create and register the notification category.
                            UNNotificationCategory *CustomizedNotification = [UNNotificationCategory
                                categoryWithIdentifier:@"CUSTOMIZED_NOTIFICATION"
                                               actions:@[]
                                     intentIdentifiers:@[]
                                               options:
                                                   UNNotificationCategoryOptionCustomDismissAction];

                            [center
                                setNotificationCategories:[NSSet
                                                              setWithObjects:CustomizedNotification,
                                                                             nil]];
                            completion_handler(0,
                                               @"User accepted request to receive notifications.");
                        }
                      }];
}

void requestNotificationPermissions(void *on_completion_callback) {
    requestNotificationPermissionsWithCompletionHandler(^(NSUInteger outcome_type, id outcome_msg) {
      dispatch_async(dispatch_get_main_queue(), ^{
        warp_on_request_notification_permissions_completed(outcome_type, outcome_msg,
                                                           on_completion_callback);
      });
    });
}

void sendNotificationWithErrorHandler(NSString *title, NSString *body, NSString *data,
                                      void (^error_handler)(NSUInteger error_type, id error_msg),
                                      BOOL playSound) {
    UNUserNotificationCenter *center = [UNUserNotificationCenter currentNotificationCenter];
    [center getNotificationSettingsWithCompletionHandler:^(UNNotificationSettings *settings) {
      if (settings.authorizationStatus == UNAuthorizationStatusDenied) {
          error_handler(0, @"User turned permissions off in system preferences.");
      } else {
          // Create the notification content.
          // `autorelease` balances the +1 retain from `alloc`; the enclosing UserNotifications
          // completion block runs on a GCD-dispatched queue that drains an ambient pool.
          UNMutableNotificationContent *content =
              [[[UNMutableNotificationContent alloc] init] autorelease];
          content.title = [NSString localizedUserNotificationStringForKey:title arguments:nil];
          content.body = [NSString localizedUserNotificationStringForKey:body arguments:nil];

          // Only play sound if the user setting allows it
          if (playSound) {
              content.sound = [UNNotificationSound defaultSound];
          }

          content.userInfo = @{
              @"DATA" : data,
          };

          // Configure the trigger to send the notification after 1 second.
          UNTimeIntervalNotificationTrigger *trigger =
              [UNTimeIntervalNotificationTrigger triggerWithTimeInterval:1 repeats:NO];

          // Create the request object.
          UNNotificationRequest *request =
              [UNNotificationRequest requestWithIdentifier:@"CUSTOMIZED_NOTIFICATION"
                                                   content:content
                                                   trigger:trigger];

          // Schedule the notification.
          [center addNotificationRequest:request
                   withCompletionHandler:^(NSError *_Nullable err) {
                     if (err != nil) {
                         error_handler(1, err.localizedDescription);
                     }
                   }];
      }
    }];
}

void sendNotification(id title, id body, id data, void *on_error_callback, BOOL playSound) {
    sendNotificationWithErrorHandler(
        title, body, data,
        ^(NSUInteger error_type, id error_msg) {
          dispatch_async(dispatch_get_main_queue(), ^{
            warp_on_notification_send_error(error_type, error_msg, on_error_callback);
          });
        },
        playSound);
}
