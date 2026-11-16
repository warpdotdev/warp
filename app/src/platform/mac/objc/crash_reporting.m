#import "crash_reporting.h"
#import <MetricKit/MetricKit.h>
#import <Sentry/Sentry-Swift.h>
#import <Sentry/Sentry.h>

void startSentry(id sentryUrl, id environment, id version, bool isDogfood) {
    [SentrySDK startWithConfigureOptions:^(SentryOptions *options) {
      options.dsn = sentryUrl;
      options.debug = NO;
      options.environment = environment;
      options.releaseName = version;
      options.enableAppHangTracking = isDogfood;
    }];
}

void stopSentry() { [SentrySDK close]; }

void crashSentry() { [SentrySDK crash]; }

void setUser(id userId) {
    SentryUser *user = [[SentryUser alloc] init];
    user.userId = userId;
    [SentrySDK setUser:user];
    // `SentrySDK setUser:` retains its own copy (per the ObjC `copy` property
    // contract on `SentryUser.userId` / `SentryScope.user`), so balance the
    // `alloc]/init]` here. Mirrors `recordBreadcrumb` below, which releases its
    // allocated `SentryBreadcrumb` after `[SentrySDK addBreadcrumb:]`.
    [user release];
}

// Define constants for the integer representations of the SentryLevel Swift
// enum.
//
// We intentionally use a different prefix (kLevel instead of kSentryLevel) to
// ensure there are no symbol name conflicts with Sentry's own code.
//
// SentryLevel is defined here:
// https://github.com/getsentry/sentry-cocoa/blob/b8ac05036d8cf7b5aa6bda6a108d7827f286ca04/Sources/Swift/Helper/Log/SentryLevel.swift#L4-L26
NSUInteger kLevelNone = 0;
NSUInteger kLevelDebug = 1;
NSUInteger kLevelInfo = 2;
NSUInteger kLevelWarning = 3;
NSUInteger kLevelError = 4;
NSUInteger kLevelFatal = 5;

// Maps the string representation of a breadcrumb level to the corresponding
// SentryLevel enum value.
//
// See Sentry-internal mapping function here:
// https://github.com/getsentry/sentry-cocoa/blob/854478ce6e1b9349d9a30c2adb59a49e80867991/Sources/Sentry/SentryLevelMapper.m
SentryLevel levelFromString(NSString *string) {
    if ([string isEqualToString:@"none"]) {
        return kLevelNone;
    }
    if ([string isEqualToString:@"debug"]) {
        return kLevelDebug;
    }
    if ([string isEqualToString:@"info"]) {
        return kLevelInfo;
    }
    if ([string isEqualToString:@"warning"]) {
        return kLevelWarning;
    }
    if ([string isEqualToString:@"error"]) {
        return kLevelError;
    }
    if ([string isEqualToString:@"fatal"]) {
        return kLevelFatal;
    }

    // Default is error, see https://develop.sentry.dev/sdk/event-payloads/#optional-attributes
    return kLevelError;
}

void recordBreadcrumb(id message, id category, id level, double seconds_since_epoch) {
    // The Rust logger may be initialized before the Sentry Cocoa SDK is enabled.
    if (![SentrySDK isEnabled]) {
        return;
    }

    SentryBreadcrumb *crumb = [[SentryBreadcrumb alloc] init];
    crumb.level = levelFromString(level);
    crumb.category = category;
    crumb.message = message;
    crumb.timestamp = [NSDate dateWithTimeIntervalSince1970:seconds_since_epoch];
    [SentrySDK addBreadcrumb:crumb];
    [crumb release];
}

void setTag(id key, id value) {
    // Set a tag on the current scope using the sentry-cocoa SDK.
    [SentrySDK configureScope:^(SentryScope *_Nonnull scope) {
      [scope setTagValue:value forKey:key];
    }];
}
