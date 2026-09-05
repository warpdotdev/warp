// Verifies that capturing a clone of the subscribed handle inside a
// ModelContext/ViewContext (4-param) subscription closure is flagged.
#![allow(dead_code)]

struct ModelHandle<T>(std::marker::PhantomData<T>);
impl<T> Clone for ModelHandle<T> {
    fn clone(&self) -> Self {
        ModelHandle(std::marker::PhantomData)
    }
}

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
    let handle_clone = handle.clone();
    ctx.subscribe_to_model(&handle, move |_me, _model, _event, _ctx| {
        let _ = &handle_clone; //~ ERROR closure captures
    });
}
