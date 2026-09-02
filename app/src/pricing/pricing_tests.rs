use warp_graphql::billing::{OveragesPricing, PricingInfo};

use super::PricingInfoModel;

#[test]
fn promotion_message_is_exposed_verbatim() {
    let model = PricingInfoModel {
        pricing_info: Some(PricingInfo {
            plans: vec![],
            overages: OveragesPricing {
                price_per_request_usd_cents: 0,
            },
            addon_credits_options: vec![],
            promotion_message: Some("50% off Fable and Opus 5".to_string()),
        }),
    };

    assert_eq!(model.promotion_message(), Some("50% off Fable and Opus 5"));
}
