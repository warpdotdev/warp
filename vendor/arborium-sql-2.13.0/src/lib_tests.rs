use super::*;

// Regression test for APP-5840: `tree_sitter_sql_external_scanner_deserialize`
// used to leak the previous `LexerState.start_tag` allocation on every
// restore of saved external-scanner state, without corrupting parse results.
// Re-parsing repeatedly against the previous tree exercises many
// serialize/deserialize round-trips of a dollar-quoted tag; this asserts
// parsing keeps succeeding, guarding against a future edit reintroducing the
// leak by breaking the free/realloc sequence in `deserialize` (e.g. by
// clearing `start_tag` unconditionally, or freeing it more than once).
#[test]
fn repeated_reparse_of_dollar_quoted_string_succeeds() {
    let mut parser = tree_sitter::Parser::new();
    parser.set_language(&language().into()).unwrap();

    let source = "CREATE FUNCTION public.foo() RETURNS trigger LANGUAGE plpgsql AS $body$ \
        BEGIN RETURN NEW; END; $body$;";

    let mut tree = parser.parse(source, None).expect("initial parse failed");
    for _ in 0..1000 {
        tree = parser.parse(source, Some(&tree)).expect("reparse failed");
    }

    assert!(!tree.root_node().has_error());
}
