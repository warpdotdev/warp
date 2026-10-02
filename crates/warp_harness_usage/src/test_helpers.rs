use serde::Serialize;
use serde_json::Value;

use crate::Findings;
use crate::api::UsagePayload;
use crate::counters::Counters;

pub(crate) fn totals<T>(payload: &UsagePayload<T>) -> Value
where
    T: Serialize + From<Counters<6>>,
    for<'a> Counters<6>: From<&'a T>,
{
    let mut counters = Counters::default();
    let mut findings = Findings::default();
    for usage in payload
        .requests
        .iter()
        .map(|row| &row.usage)
        .chain(payload.unattributed_usage.iter())
    {
        counters.add(&Counters::from(usage), &mut findings);
    }
    serde_json::to_value(T::from(counters)).unwrap()
}
