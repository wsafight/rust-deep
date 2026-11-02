trait Shape {}
trait Drawable: Shape {}

fn downcast(value: &dyn Shape) -> &dyn Drawable {
    value
}
