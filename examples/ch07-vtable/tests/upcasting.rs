use ch07_vtable::{Drawable, Shape, Sq, upcast_shape};

#[test]
fn a_subtrait_object_can_be_upcast_to_its_supertrait() {
    let square = Sq(3.0);
    let drawable: &dyn Drawable = &square;
    let shape = upcast_shape(drawable);
    assert_eq!(shape.area(), 9.0);
    assert_eq!(shape.name(), "Sq");
    assert_eq!(drawable.draw(), "square");
    assert!(std::ptr::eq(
        shape as *const dyn Shape as *const (),
        &square as *const Sq as *const ()
    ));
}
