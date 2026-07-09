use fuzzy_match::FuzzyMatchResult;
use ordered_float::OrderedFloat;
use warp_errors::report_error;

#[derive(Clone, Debug)]
pub struct FuzzyMatchExternalSecretResult {
    pub name_match_result: Option<FuzzyMatchResult>,
}

impl FuzzyMatchExternalSecretResult {
    pub fn try_match(query: &str, name: &str) -> Option<FuzzyMatchExternalSecretResult> {
        let name_match_result = fuzzy_match::match_indices_case_insensitive(name, query);
        match name_match_result {
            None => None,
            _ => Some(FuzzyMatchExternalSecretResult { name_match_result }),
        }
    }

    pub fn score(&self) -> OrderedFloat<f64> {
        let scores = self.name_match_result.iter().map(|result| result.score);

        let (sum, count) = scores.fold((0, 0), |(acc_sum, acc_count), score| {
            (acc_sum + score, acc_count + 1)
        });

        if count == 0 {
            report_error!("Secret object doesn't have a name match result.");
            OrderedFloat(f64::MIN)
        } else {
            OrderedFloat((sum / (count as i64)) as f64)
        }
    }
}
