#import "window_blur.h"

static NSString *const kApplicationServicesFramework =
    @"/System/Library/Frameworks/ApplicationServices.framework";

// Returns a function pointer to the private function named `func` in the given
// `library`. Returns NULL if the function does not exist.
static void *GetFunctionByName(NSString *library, char *func) {
    CFBundleRef bundle;
    CFURLRef bundleURL = CFURLCreateWithFileSystemPath(kCFAllocatorDefault, (CFStringRef)library,
                                                       kCFURLPOSIXPathStyle, true);
    CFStringRef functionName =
        CFStringCreateWithCString(kCFAllocatorDefault, func, kCFStringEncodingASCII);
    bundle = CFBundleCreate(kCFAllocatorDefault, bundleURL);
    void *f = NULL;
    if (bundle) {
        f = CFBundleGetFunctionPointerForName(bundle, functionName);
        CFRelease(bundle);
    }
    CFRelease(functionName);
    CFRelease(bundleURL);
    return f;
}

CGSSetWindowBackgroundBlurRadiusFunction *GetCGSSetWindowBackgroundBlurRadiusFunction(void) {
    static BOOL tried = NO;
    static CGSSetWindowBackgroundBlurRadiusFunction *function = NULL;
    if (!tried) {
        function =
            GetFunctionByName(kApplicationServicesFramework, "CGSSetWindowBackgroundBlurRadius");
        tried = YES;
    }
    return function;
}
