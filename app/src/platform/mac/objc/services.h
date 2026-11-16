// Our class for handling NSServices messages.
@interface WarpServicesProvider : NSObject
@end

// Functions implemented in Rust.
id warp_services_provider_custom_url_scheme();
void warp_app_open_urls(id app, id urls);
