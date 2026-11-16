#import <Sentry/Sentry.h>

void start(id, id, id, bool);
void setUser(id);
void recordBreadcrumb(id, id, id, double);

@interface SentryLevelMapper : NSObject

/**
 * Maps a string to a SentryLevel. If the passed string doesn't match any level this defaults to
 * the 'error' level. See https://develop.sentry.dev/sdk/event-payloads/#optional-attributes
 */
+ (SentryLevel)levelWithString:(NSString *)string;

@end
