Implements a dock tile plugin for Warp.

The plugin is used to update the dock tile icon when the app icon changes and allows for app icon changes to be persisted across app restarts. Without the plugin, the icon state reverts to the default upon the first time the app quits (although for some reason after the first quit the icon state is preserved).

The plugin is implemented in Objective-C and is built using the `clang` compiler
with the `-bundle` flag.  See the Makefile for more details.

The plugin is installed into the app bundle at `Contents/PlugIns/WarpDockTilePlugin.docktileplugin` and is bundled using the script/mac/bundle script.  It is built as a universal binary for both arm64 and x86_64.

The plugin is a simple Objective-C program that listens for notifications from the main application when the app icon changes. When it receives a notification, it updates the dock tile icon.

See Mac documentation for more details:
https://developer.apple.com/documentation/appkit/nsdocktileplugin?language=objc

Note, that during development, MacOS is not great about reloading the plugin when changes are made. 

The suggested workflow after rebuilding is to
1. Remove the icon from the dock.
2. Run `killall Dock && killall SystemUIServer`

Also, there is a sample plugin at [MacDockTileSample](https://github.com/CartBlanche/MacDockTileSample) that is helpful for installing and iterating on.

Another tip is to add file based debug logs rather than using NSLog, as it's impossible to find where NSLogs are being written to.
