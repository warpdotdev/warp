#include "WarpDockTilePlugin.h"

@implementation WarpDockTilePlugIn {
    NSFileHandle *_logFileHandle;
}

@synthesize iconChangedObserver;
@synthesize defaultsObserver;

- (void)logMessage:(NSString *)message {
    if (_logFileHandle) {
        NSDateFormatter *formatter = [[NSDateFormatter alloc] init];
        [formatter setDateFormat:@"yyyy-MM-dd HH:mm:ss.SSS"];
        NSString *timestamp = [formatter stringFromDate:[NSDate date]];
        NSString *logEntry = [NSString stringWithFormat:@"[%@] %@\n", timestamp, message];
        [_logFileHandle writeData:[logEntry dataUsingEncoding:NSUTF8StringEncoding]];
        [_logFileHandle synchronizeFile];
    }
}

- (instancetype)init {
    self = [super init];
    if (self) {
        @try {
            NSDateFormatter *formatter = [[NSDateFormatter alloc] init];
            [formatter setDateFormat:@"yyyy-MM-dd_HH-mm-ss"];
            NSString *timestamp = [formatter stringFromDate:[NSDate date]];
            NSString *logPath = [NSString stringWithFormat:@"/tmp/warp_docktile_%@.log", timestamp];
            NSError *error = nil;
            [[NSFileManager defaultManager] createFileAtPath:logPath contents:nil attributes:nil];
            _logFileHandle = [NSFileHandle fileHandleForWritingAtPath:logPath];
            [self logMessage:@"WarpDockTilePlugin initialized"];
        } @catch (NSException *exception) {
            NSLog(@"Exception during initialization: %@\nStack trace: %@", 
                  exception.reason, 
                  exception.callStackSymbols);
        }
    }
    return self;
}

- (void)updateAppIcon:(NSDockTile *)tile {
    @try {
        [self logMessage:@"updateAppIcon called"];
        // Retrieve the bundle ID for the main app from the Info.plist file in the plugin bundle
        NSBundle *pluginBundle = [NSBundle bundleForClass:[self class]];    
        NSString *path = [[pluginBundle bundlePath] stringByAppendingPathComponent:@"Contents/Info.plist"];
        NSDictionary *dict = [NSDictionary dictionaryWithContentsOfFile:path];    
        NSString *bundleId = dict[@"MainAppBundleIdentifier"];    
        BOOL isDev = [bundleId containsString:@"Dev"];
        BOOL isPreview = [bundleId containsString:@"Preview"];
        BOOL isLocal = [bundleId containsString:@"Local"];
        [self logMessage:[NSString stringWithFormat:@"Plugin Bundle ID: %@", bundleId]];

        // Initialize the user defaults for the main app
        NSUserDefaults *hostDefaults = [[NSUserDefaults alloc] initWithSuiteName:bundleId];
        [hostDefaults synchronize];

        // Get the icon name from the user defaults
        NSString* appIconName = [hostDefaults stringForKey:@"AppIcon"];

        // Check if the user has set a non-default icon. If the AppIcon key is nil, empty, or "Default", reset to the
        // icon bundled with the app by setting the content to "nil". Using the icon bundled in the app will allow macOS
        // to handle the icon, including respecting "Icon & Widget Style" setting and applying a color filter. Non-
        // default icons do not respect that setting.
        NSString* cleanName = [[appIconName stringByTrimmingCharactersInSet:[NSCharacterSet characterSetWithCharactersInString:@"\""]] lowercaseString];
        if (!appIconName || [appIconName length] == 0 || [cleanName isEqualToString:@"default"]) {
            [self logMessage:@"User has default icon, resetting dock tile to system default"];
            [tile setContentView:nil];
            [tile display];
            return;
        }

        NSString* iconFileName = [self convertAppIconNameToFileName:appIconName isDev:isDev isLocal:isLocal isPreview:isPreview];
        [self logMessage:[NSString stringWithFormat:@"Icon file name: %@", iconFileName]];

        // Load the icon image
        NSImage* currentImage = [self LoadDockTileImage:iconFileName];

        // Set the image on the dock tile
        // 256 x 256 is the preferred size for retina dock icons.
        NSImageView* imageView = [[NSImageView alloc] initWithFrame:NSMakeRect(0, 0, 256, 256)];
        [imageView setImage:currentImage];
        [imageView setImageScaling:NSImageScaleProportionallyUpOrDown];
        [tile setContentView:imageView];
        [tile display];
        
        [self logMessage:[NSString stringWithFormat:@"Dock tile updated with icon: %@", iconFileName]];
    } @catch (NSException *exception) {
        [self logMessage:[NSString stringWithFormat:@"Exception updating dock tile icon: %@\nStack trace: %@\nTile: %@", 
              exception.reason, 
              exception.callStackSymbols,
              tile ? @"valid" : @"nil"]];
    }
}

// See app_icon.rs for the rust version of this conversion.
- (NSString*)convertAppIconNameToFileName:(NSString*)appIconName isDev:(BOOL)isDev isLocal:(BOOL)isLocal isPreview:(BOOL)isPreview {
    // First remove quotes and convert to lowercase
    NSString* cleanName = [[appIconName stringByTrimmingCharactersInSet:[NSCharacterSet characterSetWithCharactersInString:@"\""]] lowercaseString];
    
    NSDictionary* mapping = @{
        @"aurora": @"aurora",
        @"classic1": @"classic_1",
        @"classic2": @"classic_2",
        @"classic3": @"classic_3",
        @"comets": @"comets",
        @"glasssky": @"glass_sky",
        @"glitch": @"glitch",
        @"glow": @"glow",
        @"holographic": @"holographic",
        @"mono": @"mono",
        @"neon": @"neon",
        @"original": @"original",
        @"starburst": @"starburst",
        @"sticker": @"sticker",
        @"warpone": @"blue",
        @"cow": @"cow"
    };
    
    NSString* fileName = mapping[cleanName];

    // If the mapping doesn't exist, return the default icon 
    // conditional on whether this is a local, dev, or preview build.
    return fileName ?: isLocal ? @"local" : isDev ? @"dev" : isPreview ? @"preview" : @"warp_2";
}

// Helper function to load named image from the plugin's resource bundle
- (NSImage*)LoadDockTileImage:(NSString*)imageName {
    NSBundle* pluginBundle = [NSBundle bundleForClass:[self class]];
    NSString* imagePath = [pluginBundle pathForResource:imageName ofType:@"png"];
    [self logMessage:[NSString stringWithFormat:@"Image path: %@", imagePath]];
    if (imagePath == nil) {
        [self logMessage:[NSString stringWithFormat:@"Could not find image named %@ in the plugin resources", imageName]];
        return nil;
    }
    return [[NSImage alloc] initWithContentsOfFile:imagePath];
}

// Protocol method that is invoked by the system when the dock for Warp is updated.
// Note that we listen for direct changes to the AppIcon key in the user defaults.
- (void)setDockTile:(NSDockTile *)dockTile {
    @try {
        [self logMessage:[NSString stringWithFormat:@"setDockTile called with tile: %@", dockTile ? @"valid" : @"nil"]];
        if (dockTile) {
            // Get the bundle ID for setting up user defaults observation
            NSBundle *pluginBundle = [NSBundle bundleForClass:[WarpDockTilePlugIn class]];    
            NSString *path = [[pluginBundle bundlePath] stringByAppendingPathComponent:@"Contents/Info.plist"];
            NSDictionary *dict = [NSDictionary dictionaryWithContentsOfFile:path];    
            NSString *bundleId = dict[@"MainAppBundleIdentifier"];

            [self logMessage:[NSString stringWithFormat:@"Main app bundleId: %@", bundleId]];

            // Set up user defaults observer
            NSUserDefaults *hostDefaults = [[NSUserDefaults alloc] initWithSuiteName:bundleId];
            [hostDefaults addObserver:self
                        forKeyPath:@"AppIcon"
                            options:NSKeyValueObservingOptionNew
                            context:(__bridge void * _Nullable)(dockTile)];
            self.defaultsObserver = hostDefaults;

            [self logMessage:[NSString stringWithFormat:@"Host defaults: %@", hostDefaults]];

            // Make sure the icon is updated from the get-go as well.
            [self updateAppIcon:dockTile];	        
        } else {
            [self logMessage:@"No docktile, clearing icon observer"];
            [[NSDistributedNotificationCenter defaultCenter] removeObserver:self.iconChangedObserver];
            self.iconChangedObserver = nil;
            if (self.defaultsObserver) {
                [(NSUserDefaults *)self.defaultsObserver removeObserver:self forKeyPath:@"AppIcon"];
                self.defaultsObserver = nil;
            }
        }
    } @catch (NSException *exception) {
        [self logMessage:[NSString stringWithFormat:@"Exception in setDockTile: %@\nStack trace: %@\nDockTile: %@", 
              exception.reason, 
              exception.callStackSymbols,
              dockTile ? @"valid" : @"nil"]];
    }
}

- (void)dealloc {
    @try {
        [self logMessage:@"WarpDockTilePlugin deallocating"];
        if (self.iconChangedObserver) {
            [[NSDistributedNotificationCenter defaultCenter] removeObserver:self.iconChangedObserver];
            self.iconChangedObserver = nil;
        }
        if (self.defaultsObserver) {
            [(NSUserDefaults *)self.defaultsObserver removeObserver:self forKeyPath:@"AppIcon"];
            self.defaultsObserver = nil;
        }
        
        if (_logFileHandle) {
            [self logMessage:@"Closing log file"];
            [_logFileHandle closeFile];
            _logFileHandle = nil;
        }
    } @catch (NSException *exception) {
        NSLog(@"Exception during deallocation: %@\nStack trace: %@", 
              exception.reason, 
              exception.callStackSymbols);
    }
}

// KVO callback method
- (void)observeValueForKeyPath:(NSString *)keyPath
                    ofObject:(id)object
                        change:(NSDictionary<NSKeyValueChangeKey,id> *)change
                    context:(void *)context {
    @try {
        if ([keyPath isEqualToString:@"AppIcon"]) {
            [self logMessage:@"AppIcon value changed in user defaults"];
            NSDockTile *dockTile = (__bridge NSDockTile *)context;
            [self updateAppIcon:dockTile];
        }
    } @catch (NSException *exception) {
        [self logMessage:[NSString stringWithFormat:@"Exception in KVO handler: %@\nStack trace: %@\nKeyPath: %@\nObject: %@\nChange: %@", 
              exception.reason, 
              exception.callStackSymbols,
              keyPath,
              object,
              change]];
    }
}

@end
