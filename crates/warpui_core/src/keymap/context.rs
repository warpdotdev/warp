use std::collections::{HashMap, HashSet};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Context {
    pub set: HashSet<&'static str>,
    pub map: HashMap<&'static str, &'static str>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum ContextPredicate {
    Identifier(&'static str),
    Equal(&'static str, &'static str),
    NotEqual(&'static str, &'static str),
    Not(Box<ContextPredicate>),
    And(Box<ContextPredicate>, Box<ContextPredicate>),
    Or(Box<ContextPredicate>, Box<ContextPredicate>),
    Just(bool),
}

pub mod macros {
    /// Returns a context predicate identifier.
    #[macro_export]
    macro_rules! id {
        ($val:literal) => {
            $crate::keymap::ContextPredicate::Identifier($val)
        };
        ($val:expr) => {
            $crate::keymap::ContextPredicate::Identifier($val)
        };
    }
    pub use id;

    /// Returns a context predicate which checks whether the given context
    /// key has a particular value.
    #[macro_export]
    macro_rules! eq {
        ($a:literal, $b:literal) => {
            $crate::keymap::ContextPredicate::Equal($a, $b)
        };
    }
    pub use eq;

    /// Returns a context predicate which checks whether the given context
    /// key does _not_ have a particular value.
    #[macro_export]
    macro_rules! ne {
        ($a:literal, $b:literal) => {
            $crate::keymap::ContextPredicate::NotEqual($a, $b)
        };
    }
    pub use ne;

    impl std::ops::Not for ContextPredicate {
        type Output = ContextPredicate;

        fn not(self) -> Self::Output {
            ContextPredicate::Not(Box::new(self))
        }
    }

    impl std::ops::BitAnd for ContextPredicate {
        type Output = ContextPredicate;

        fn bitand(self, rhs: Self) -> Self::Output {
            ContextPredicate::And(Box::new(self), Box::new(rhs))
        }
    }

    impl std::ops::BitOr for ContextPredicate {
        type Output = ContextPredicate;

        fn bitor(self, rhs: Self) -> Self::Output {
            ContextPredicate::Or(Box::new(self), Box::new(rhs))
        }
    }

    /// Returns a context predicate which is always matched.
    #[macro_export]
    macro_rules! always {
        () => {
            $crate::keymap::ContextPredicate::Just(true)
        };
    }
    pub use always;

    use super::ContextPredicate;
}

impl Context {
    pub fn extend(&mut self, other: Context) {
        for v in other.set {
            self.set.insert(v);
        }
        for (k, v) in other.map {
            self.map.insert(k, v);
        }
    }
}

impl ContextPredicate {
    pub fn eval(&self, ctx: &Context) -> bool {
        match self {
            Self::Identifier(name) => ctx.set.contains(*name),
            Self::Equal(left, right) => ctx
                .map
                .get(left)
                .map(|value| value == right)
                .unwrap_or(false),
            Self::NotEqual(left, right) => ctx
                .map
                .get(left)
                .map(|value| value != right)
                .unwrap_or(true),
            Self::Not(pred) => !pred.eval(ctx),
            Self::And(left, right) => left.eval(ctx) && right.eval(ctx),
            Self::Or(left, right) => left.eval(ctx) || right.eval(ctx),
            Self::Just(val) => *val,
        }
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
