// Verifies that capturing a clone of the subscribed handle inside an
// AppContext (3-param) subscription closure is flagged.
#![allow(dead_code)]

struct ModelHandle<T>(std::marker::PhantomData<T>);
impl<T> Clone for ModelHandle<T> {
    fn clone(&self) -> Self {
        ModelHandle(std::marker::PhantomData)
    }
}

struct AppContext;
impl AppContext {
    fn subscribe_to_model<E, F>(&mut self, _handle: &ModelHandle<E>, _callback: F)
    where
        F: FnMut(ModelHandle<E>, &(), &mut AppContext),
    {
    }
}

struct Foo;

fn main() {
    let mut ctx = AppContext;
    let handle = ModelHandle::<Foo>(std::marker::PhantomData);
    let handle_clone = handle.clone();
    ctx.subscribe_to_model(&handle, move |_handle, _event, _ctx| {
        let _ = &handle_clone; //~ ERROR closure captures
    });
}
