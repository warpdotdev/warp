BUNDLE_NAME = WarpDockTilePlugin.docktileplugin
OBJC_FILES = WarpDockTilePlugin.m
FRAMEWORKS = -framework Cocoa -framework AppKit -framework Foundation
# Compile a universal binary for both arm64 and x86_64.
CFLAGS = -fobjc-arc -bundle -mmacosx-version-min=$(MACOSX_DEPLOYMENT_TARGET) -arch arm64 -arch x86_64
LDFLAGS = -ObjC

.PHONY: all clean

all: $(BUNDLE_NAME)

$(BUNDLE_NAME): $(OBJC_FILES)
	mkdir -p $(BUNDLE_NAME)/Contents/MacOS
	clang $(CFLAGS) $(LDFLAGS) $(FRAMEWORKS) $(OBJC_FILES) -o $(BUNDLE_NAME)/Contents/MacOS/WarpDockTilePlugin
	cp Info.plist $(BUNDLE_NAME)/Contents/
	mkdir -p $(BUNDLE_NAME)/Contents/Resources
	cp Resources/* $(BUNDLE_NAME)/Contents/Resources/

clean:
	rm -rf $(BUNDLE_NAME)
