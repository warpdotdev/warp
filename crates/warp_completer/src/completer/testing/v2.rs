use async_trait::async_trait;
use warp_js::{JsFunctionId, SerializedJsValue};

use crate::completer::context::{JsExecutionContext, JsExecutionError};
use crate::signatures::testing::{TEST_GENERATOR_1_JS_FUNCTION, TEST_GENERATOR_2_JS_FUNCTION};
use crate::signatures::{GeneratorResults, Suggestion};

pub struct FakeJsExecutionContext {}

#[async_trait]
impl JsExecutionContext for FakeJsExecutionContext {
    async fn call_js_function(
        &self,
        _input: SerializedJsValue,
        function_id: JsFunctionId,
    ) -> Result<SerializedJsValue, JsExecutionError> {
        let results = match function_id {
            id if TEST_GENERATOR_1_JS_FUNCTION.id == id => GeneratorResults {
                suggestions: vec![
                    Suggestion {
                        value: "foo".to_owned(),
                        ..Default::default()
                    },
                    Suggestion {
                        value: "bar".to_owned(),
                        ..Default::default()
                    },
                ],
                is_ordered: false,
            },
            id if TEST_GENERATOR_2_JS_FUNCTION.id == id => GeneratorResults {
                suggestions: vec![
                    Suggestion {
                        value: "def".to_owned(),
                        ..Default::default()
                    },
                    Suggestion {
                        value: "abc".to_owned(),
                        ..Default::default()
                    },
                ],
                is_ordered: true,
            },
            _ => panic!("Unexpected JS function call!"),
        };
        SerializedJsValue::from_value(results).map_err(JsExecutionError::Serialization)
    }
}
