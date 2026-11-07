use gpui_kit::{canvas, App, Bounds, Canvas, Context, Entity, Pixels, Styled, Window};

pub enum BoundsValue<B: From<Bounds<Pixels>> + 'static> {
    Entity(Entity<B>),
    Callback(Box<dyn Fn(Bounds<Pixels>, &mut Window, &mut App)>),
}

fn generic_bounds<B>(bounds: BoundsValue<B>) -> Canvas<()>
where
    B: From<Bounds<Pixels>> + 'static,
{
    canvas(
        move |canvas_bounds, window, cx| match bounds {
            BoundsValue::Entity(bounds) => bounds.update(cx, |bounds, cx| {
                *bounds = canvas_bounds.into();
                cx.notify();
            }),
            BoundsValue::Callback(callback) => callback(canvas_bounds.into(), window, cx),
        },
        |_, _, _, _| {},
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

pub fn bounds<B>(bounds: &Entity<B>) -> Canvas<()>
where
    B: From<Bounds<Pixels>> + 'static,
{
    generic_bounds(BoundsValue::Entity(bounds.clone()))
}

pub fn bounds_callback(
    callback: impl Fn(Bounds<Pixels>, &mut Window, &mut App) + 'static,
) -> Canvas<()> {
    generic_bounds::<Bounds<Pixels>>(BoundsValue::Callback(Box::new(callback)))
}

pub trait ContextExt<'a, C> {
    fn bounds(
        &mut self,
        callback: impl Fn(&mut C, Bounds<Pixels>, &mut Window, &mut Context<C>) + 'static,
    ) -> Canvas<()>;
}

impl<'a, C: 'static> ContextExt<'a, C> for Context<'a, C> {
    fn bounds(
        &mut self,
        callback: impl Fn(&mut C, Bounds<Pixels>, &mut Window, &mut Context<C>) + 'static,
    ) -> Canvas<()> {
        bounds_callback(self.processor(move |this, bounds, window, cx| {
            callback(this, bounds, window, cx);
        }))
    }
}
