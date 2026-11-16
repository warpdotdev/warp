use super::*;

#[test]
fn test_sentence_parser() {
    assert_eq!(
        parse_query_into_tokens("This is a question?"),
        vec![
            "This".to_string(),
            "is".to_string(),
            "a".to_string(),
            "question".to_string()
        ]
    );

    assert_eq!(
        parse_query_into_tokens("No I can't!"),
        vec!["No".to_string(), "I".to_string(), "can't".to_string()]
    );

    assert_eq!(
        parse_query_into_tokens("A quote \"Inside quote\""),
        vec![
            "A".to_string(),
            "quote".to_string(),
            "\"Inside quote\"".to_string()
        ]
    );

    assert_eq!(
        parse_query_into_tokens("A quote \"Inside ' quote\""),
        vec![
            "A".to_string(),
            "quote".to_string(),
            "\"Inside ' quote\"".to_string()
        ]
    );

    assert_eq!(
        parse_query_into_tokens("A quote \"Inside 'something' quote\""),
        vec![
            "A".to_string(),
            "quote".to_string(),
            "\"Inside 'something' quote\"".to_string()
        ]
    );

    assert_eq!(
        parse_query_into_tokens("Empty quote \"\"!?!"),
        vec!["Empty".to_string(), "quote".to_string()]
    );

    assert_eq!(
        parse_query_into_tokens("www.google.com"),
        vec!["www.google.com".to_string(),]
    );

    assert_eq!(
        parse_query_into_tokens("Command `mockery --name example_interface`"),
        vec![
            "Command".to_string(),
            "`mockery --name example_interface`".to_string()
        ]
    );
}
