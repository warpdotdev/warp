// Verifies that using the provided callback parameter instead of capturing
// an owned clone does not trigger the lint.
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

fn main() {
    let mut ctx = ModelContext::<Foo>(std::marker::PhantomData);
    let handle = ModelHandle::<Foo>(std::marker::PhantomData);
    // Uses the provided second parameter — no capture, no error.
    ctx.subscribe_to_model(&handle, move |_me, model, _event, _ctx| {
        let _ = model;
    });
}
