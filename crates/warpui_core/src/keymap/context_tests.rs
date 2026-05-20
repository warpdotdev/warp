use super::{macros::*, *};

#[test]
fn test_context_predicate_eval() -> anyhow::Result<()> {
    let predicate = id!("a") & id!("b") | eq!("c", "d");

    let mut context = Context::default();
    context.set.insert("a");
    assert!(!predicate.eval(&context));

    context.set.insert("b");
    assert!(predicate.eval(&context));

    context.set.remove("b");
    context.map.insert("c", "x");
    assert!(!predicate.eval(&context));

    context.map.insert("c", "d");
    assert!(predicate.eval(&context));

    let predicate = !id!("a");
    assert!(predicate.eval(&Context::default()));

    Ok(())
}
