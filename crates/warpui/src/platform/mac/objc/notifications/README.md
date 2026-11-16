# MacOS User Notifcations

The Apple framework we use to support notifications is `UNUserNotifications`: https://developer.apple.com/documentation/usernotifications?language=objc

## Developing notifications locally
The framework needs a signed app to be able to request authorization and schedule notifications to Apple's Notification Center. For this reason, it is not enough to `cargo build && cargo run`. Instead, there are a couple of options:

### 1. Bundle the app
This takes a longer time than option 2, but it is more stable, and it is what the user will ultimately experience.

1. Run `script/user_notifications --nouniversal --open`

If you want to test the authorization flow specifically, you will have to:
1. Delete all the `WarpDev` apps you have installed locally
2. Ensure that `WarpDev` isn't an app in your Notification Center by checking which apps show up in `Notifications` in your System Preferences.
3. Log out*
4. Log back in and bundle&run the app again.

### 2. Nosign the local build (script)
This is less stable than option 1 and is not recommended if you're testing the authorization flow.

1. Ensure that you have a WarpDev app installed, _and in your Applications folder_. It's important to have the app in your Applications, or Apple won't be able to find the app while testing notifications.
2. Run `script/local_build_and_sign`

If you want to test the authorization flow specifically, you will have to:
1. Delete all the `WarpDev` apps you have installed locally, including the one in your Applications folder.
2. Ensure that `WarpDev` isn't an app in your Notification Center by checking which apps show up in `Notifications` in your System Preferences.
3. Log out* and log back in.
4. Move the `WarpDev` from `Bin` to `Applications` (i.e. install `WarpDev`).
5. Run the script to nosign and run the app again: `script/local_build_and_sign`.

*NB: If you already have all the permissions to send notifications, and you're not testing the authorization flow, you should be able to do the following instead of logging out & in:
1. Delete all `WarpDev` apps.
2. Run `sudo lsof | grep usernoted | grep db2` to find the path to a database that Notification Center uses.
3. Run `killall usernoted && killall NotificationCenter`
4. Run `rm <path-to-notification-center-db>`
5. Build and run the app again (if you're not bundling, you still have to move `WarpDev` back to your `Applications` folder).

## Debugging notifications
Some useful methods for debugging errors / if things aren't working as expected:
- Check Notification Center to figure out if `WarpDev` is a registered app and to play around with the settings (e.g. turning on/off, enabling sound, etc)
- Use `NSLog` to print debug statements in local builds
- Use the Console app for more helpful framework errors - this is particularly helpful when you don't see any error from your own logs. Filter the messages by `NotificationCenter` or `usernoted` or `dev`
- When in doubt, delete all `WarpDev` apps and restart your laptop. Sometimes Notification Center needs a gentle nudge.
