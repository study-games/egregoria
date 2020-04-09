use crate::interaction::Selectable;
use crate::physics::{Collider, CollisionWorld, Kinematics, PhysicsObject, Transform};
use crate::rendering::meshrender_component::{CircleRender, MeshRender};
use cgmath::vec2;
use specs::{Builder, World, WorldExt};

pub mod data;
pub mod systems;

pub use data::*;
pub use systems::*;

pub fn setup(world: &mut World) {
    for _ in 0..1000 {
        spawn_pedestrian(world);
    }
}

pub fn spawn_pedestrian(world: &mut World) {
    let pos = 200.0f32 * vec2(rand::random(), rand::random());
    let h = world
        .get_mut::<CollisionWorld>()
        .unwrap()
        .insert(pos, PhysicsObject::with_radius(0.5));

    let x_obj = if rand::random() {
        20.0f32 * rand::random::<f32>()
    } else {
        200.0 - 20.0f32 * rand::random::<f32>()
    };

    world
        .create_entity()
        .with(Transform::new(pos))
        .with(PedestrianComponent {
            objective: vec2(x_obj, pos.y),
            ..Default::default()
        })
        .with(Kinematics::from_mass(80.0))
        .with(MeshRender::simple(
            CircleRender {
                radius: 0.5,
                ..Default::default()
            },
            3,
        ))
        .with(Collider(h))
        .with(Selectable::new(0.5))
        .build();
}