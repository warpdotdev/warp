// Verifies that capturing a handle of a DIFFERENT type than the subscribed
// handle is not flagged (cross-entity captures are intentionally allowed).
#![allow(dead_code)]

struct ModelHandle<T>(std::marker::PhantomData<T>);

struct ModelContext<T>(std::marker::PhantomData<T>);
impl<T> ModelContext<T> {
    fn subscribe_to_model<E, F>(&mut self, _handle: &ModelHandle<E>, _callback: F)
    where
        F: FnMut(&mut T, ModelHandle<E>, &(), &mut ModelContext<T>),
    {
    }
}

struct Foo;
struct Bar;

fn main() {
    let mut ctx = ModelContext::<Foo>(std::marker::PhantomData);
    let foo_handle = ModelHandle::<Foo>(std::marker::PhantomData);
    let bar_handle = ModelHandle::<Bar>(std::marker::PhantomData);
    // bar_handle is a different entity type — no error expected.
    ctx.subscribe_to_model(&foo_handle, move |_me, _model, _event, _ctx| {
        let _ = &bar_handle;
    });
}
