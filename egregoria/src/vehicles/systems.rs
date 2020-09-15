use crate::engine_interaction::TimeInfo;
use crate::map_dynamic::{Itinerary, ParkingManagement, OBJECTIVE_OK_DIST};
use crate::physics::Kinematics;
use crate::physics::{Collider, CollisionWorld, PhysicsGroup, PhysicsObject};
use crate::utils::Restrict;
use crate::vehicles::{Vehicle, VehicleState, TIME_TO_PARK};
use crate::{Deleted, ParCommandBuffer};
use geom::{angle_lerp, Vec2};
use geom::{both_dist_to_inter, Ray};
use geom::{Spline, Transform};
use legion::system;
use legion::Entity;
use map_model::{Map, TrafficBehavior, Traversable, TraverseKind};

#[system]
pub fn vehicle_cleanup(
    #[resource] evts: &mut Deleted<Vehicle>,
    #[resource] pm: &mut ParkingManagement,
) {
    for comp in evts.drain() {
        if let Some(id) = comp.park_spot {
            pm.free(id)
        }
    }
}

#[system(for_each)]
pub fn vehicle_decision(
    #[resource] map: &Map,
    #[resource] time: &TimeInfo,
    #[resource] cow: &CollisionWorld,
    it: &mut Itinerary,
    trans: &mut Transform,
    kin: &mut Kinematics,
    vehicle: &mut Vehicle,
    collider: &Collider,
) {
    let (_, self_obj) = cow.get(collider.0).expect("Handle not in collision world");
    let danger_length = (self_obj.speed.powi(2) / (2.0 * vehicle.kind.deceleration())).min(40.0);
    let neighbors = cow.query_around(trans.position(), 12.0 + danger_length);
    let objs = neighbors.map(|(id, pos)| {
        (
            Vec2::from(pos),
            cow.get(id).expect("Handle not in collision world").1,
        )
    });

    let (desired_speed, desired_dir) =
        calc_decision(vehicle, &map, &time, trans, self_obj, it, objs);

    physics(
        trans,
        kin,
        vehicle,
        &time,
        self_obj,
        &map,
        desired_speed,
        desired_dir,
    );
}

/// Decides whether a vehicle should change states, from parked to unparking to driving etc
#[system(for_each)]
pub fn vehicle_state_update(
    #[resource] buf: &ParCommandBuffer,
    #[resource] map: &Map,
    #[resource] time: &TimeInfo,
    trans: &Transform,
    vehicle: &mut Vehicle,
    kin: &mut Kinematics,
    it: &mut Itinerary,
    ent: &Entity,
) {
    let trans = *trans;
    let ent = *ent;

    match vehicle.state {
        VehicleState::RoadToPark(_, ref mut t) => {
            // Vehicle is on rails when parking.

            *t += time.delta / TIME_TO_PARK;

            if *t >= 1.0 {
                let spot = unwrap_or!(vehicle.park_spot, {
                    vehicle.state = VehicleState::Driving;
                    return;
                });
                buf.remove_component::<Collider>(ent);
                kin.velocity = Vec2::ZERO;

                vehicle.state = VehicleState::Parked(spot);
            }
        }
        VehicleState::Driving => {
            if it.has_ended(time.time) {
                *it = Itinerary::wait_until(time.time + 20.0);
                let spot = vehicle.park_spot.and_then(|id| map.parking.get(id));

                let spot = unwrap_or!(spot, return);

                let s = Spline {
                    from: trans.position(),
                    to: spot.pos,
                    from_derivative: trans.direction() * 2.0,
                    to_derivative: spot.orientation * 2.0,
                };

                vehicle.state = VehicleState::RoadToPark(s, 0.0);
                kin.velocity = Vec2::ZERO;
            }
        }
        VehicleState::Parked(_) => {
            // Wait until it's time to start driving again, then set a route and unpark.
            /*
            if it.has_ended(time.time) {
                let mut lane = map.parking_to_drive(spot);

                if lane.is_none() {
                    lane = map.nearest_lane(trans.position(), LaneKind::Driving);
                }

                let travers: Option<Traversable> = lane
                    .map(|x| Traversable::new(TraverseKind::Lane(x), TraverseDirection::Forward));

                if let Some((mut itin, park)) =
                    next_objective(trans.position(), parking, map, travers.as_ref())
                {
                    parking.free(spot);

                    let points = itin.get_travers().unwrap().points(map).unwrap(); // Unwraps ok: just got itinerary
                    let d = points.distance_along(points.project(trans.position()));

                    let (pos, dir) = points.point_dir_along(d + 5.0);

                    let s = Spline {
                        from: trans.position(),
                        to: pos,
                        from_derivative: trans.direction() * 2.0,
                        to_derivative: dir * 2.0,
                    };

                    // Create some points along the spline and repack the itin with the new points.
                    itin.prepend_local_path(s.split_at(0.8).0.points(8).collect());

                    let w = vehicle.kind.width();
                    buf.exec(move |goria| {
                        let mut cow = goria.write::<CollisionWorld>();
                        let h = Collider(cow.insert(
                            pos,
                            PhysicsObject {
                                dir: trans.direction(),
                                group: PhysicsGroup::Vehicles,
                                radius: w * 0.5,
                                speed: 0.0,
                            },
                        ));
                        drop(cow);

                        if let Some(mut v) = goria.world.entry(ent) {
                            v.add_component(h);
                        }
                    });

                    *it = itin;
                    vehicle.park_spot = Some(park);
                    vehicle.state = VehicleState::Driving;
                } else {
                    *it = Itinerary::wait_until(time.time + 10.0);
                }
            }
             */
        }
    }
}

/// Handles actually moving the vehicles around, including acceleration and other physics stuff.
fn physics(
    trans: &mut Transform,
    kin: &mut Kinematics,
    vehicle: &mut Vehicle,
    time: &TimeInfo,
    obj: &PhysicsObject,
    map: &Map,
    desired_speed: f32,
    desired_dir: Vec2,
) {
    match vehicle.state {
        VehicleState::Parked(id) => {
            let spot = unwrap_or!(map.parking.get(id), return);
            trans.set_position(spot.pos);
            trans.set_direction(spot.orientation);
            return;
        }
        VehicleState::RoadToPark(spline, t) => {
            trans.set_position(spline.get(t));
            trans.set_direction(spline.derivative(t).normalize());
            return;
        }
        VehicleState::Driving => {}
    }

    let speed = obj.speed;
    let kind = vehicle.kind;
    let direction = trans.direction();

    let speed = speed
        + (desired_speed - speed).restrict(
            -time.delta * kind.deceleration(),
            time.delta * kind.acceleration(),
        );

    let max_ang_vel = (speed.abs() / kind.min_turning_radius()).restrict(0.0, 2.0);

    let approx_angle = direction.distance(desired_dir);

    vehicle.ang_velocity += time.delta * kind.ang_acc();
    vehicle.ang_velocity = vehicle
        .ang_velocity
        .min(3.0 * approx_angle)
        .min(max_ang_vel);

    trans.set_direction(angle_lerp(
        trans.direction(),
        desired_dir,
        vehicle.ang_velocity * time.delta,
    ));

    kin.velocity = trans.direction() * speed;
}

/// Decide the appropriate velocity and direction to aim for.
pub fn calc_decision<'a>(
    vehicle: &mut Vehicle,
    map: &Map,
    time: &TimeInfo,
    trans: &Transform,
    self_obj: &PhysicsObject,
    it: &Itinerary,
    neighs: impl Iterator<Item = (Vec2, &'a PhysicsObject)>,
) -> (f32, Vec2) {
    let default_return = (0.0, self_obj.dir);
    if vehicle.wait_time > 0.0 {
        vehicle.wait_time -= time.delta;
        return default_return;
    }
    let objective: Vec2 = unwrap_or!(it.get_point(), return default_return);

    let terminal_pos = it.get_terminal();

    let front_dist = calc_front_dist(vehicle, trans, self_obj, it, neighs);

    let position = trans.position();
    let speed = self_obj.speed;
    if speed.abs() < 0.2 && front_dist < 1.5 {
        vehicle.wait_time = (position.x * 1000.0).fract().abs() * 0.5;
        return default_return;
    }

    let dir_to_pos = unwrap_or!(
        (objective - position).try_normalize(),
        return default_return
    );

    let time_to_stop = speed / vehicle.kind.deceleration();
    let stop_dist = time_to_stop * speed * 0.5;

    if let Some(pos) = terminal_pos {
        // Close to terminal objective
        if pos.distance(trans.position()) < 1.0 + stop_dist {
            return (0.0, dir_to_pos);
        }
    }

    if let Some(Traversable {
        kind: TraverseKind::Lane(l_id),
        ..
    }) = it.get_travers()
    {
        if let Some(l) = map.lanes().get(*l_id) {
            let dist_to_light = l.control_point().distance(position);
            match l.control.get_behavior(time.time_seconds) {
                TrafficBehavior::RED | TrafficBehavior::ORANGE => {
                    if dist_to_light
                        < OBJECTIVE_OK_DIST * 1.05
                            + 2.0
                            + stop_dist
                            + (vehicle.kind.width() * 0.5 - OBJECTIVE_OK_DIST).max(0.0)
                    {
                        return (0.0, dir_to_pos);
                    }
                }
                TrafficBehavior::STOP => {
                    if dist_to_light < OBJECTIVE_OK_DIST * 0.95 + stop_dist {
                        return (0.0, dir_to_pos);
                    }
                }
                _ => {}
            }
        }
    }

    // Stop at 80 cm of object in front
    if front_dist < 0.8 + stop_dist {
        return (0.0, dir_to_pos);
    }

    // Not facing the objective
    if dir_to_pos.dot(trans.direction()) < 0.8 {
        return (6.0, dir_to_pos);
    }

    (vehicle.kind.cruising_speed(), dir_to_pos)
}

/// Calculates the distance to the closest problematic object in front of the car.
/// It can be another car or a pedestrian, or it can be a potential collision point from a
/// car coming perpendicularly.
fn calc_front_dist<'a>(
    vehicle: &mut Vehicle,
    trans: &Transform,
    self_obj: &PhysicsObject,
    it: &Itinerary,
    neighs: impl Iterator<Item = (Vec2, &'a PhysicsObject)>,
) -> f32 {
    let position = trans.position();
    let direction = trans.direction();

    let mut min_front_dist: f32 = 50.0;

    let my_ray = Ray {
        from: position - direction * vehicle.kind.width() * 0.5,
        dir: direction,
    };

    let my_radius = self_obj.radius;
    let speed = self_obj.speed;

    let on_lane = it.get_travers().map_or(false, |t| t.kind.is_lane());

    // Collision avoidance
    for (his_pos, nei_physics_obj) in neighs {
        // Ignore myself
        if std::ptr::eq(nei_physics_obj, self_obj) {
            continue;
        }

        let towards_vec: Vec2 = his_pos - position;
        let (towards_dir, dist) = unwrap_or!(towards_vec.dir_dist(), continue);

        // cos of angle from self to obj
        let cos_angle = towards_dir.dot(direction);

        // Ignore things behind
        if cos_angle < 0.0 {
            continue;
        }

        let dist_to_side = towards_vec.perp_dot(direction).abs();

        let is_vehicle = matches!(nei_physics_obj.group, PhysicsGroup::Vehicles);

        let cos_direction_angle = nei_physics_obj.dir.dot(direction);

        // front cone
        if cos_angle > 0.85 - 0.015 * speed.min(10.0)
            && (!is_vehicle || cos_direction_angle > 0.0)
            && (!on_lane || dist_to_side < 3.0)
        {
            let mut dist_to_obj = dist - my_radius - nei_physics_obj.radius;
            if !is_vehicle {
                dist_to_obj -= 1.0;
            }
            min_front_dist = min_front_dist.min(dist_to_obj);
            continue;
        }

        // don't do ray checks for other things than cars
        if !is_vehicle {
            continue;
        }

        // closest win
        let his_ray = Ray {
            from: his_pos - nei_physics_obj.radius * nei_physics_obj.dir,
            dir: nei_physics_obj.dir,
        };

        let (my_dist, his_dist) = unwrap_or!(both_dist_to_inter(my_ray, his_ray), continue);

        if my_dist - speed.min(2.5) - my_radius
            < his_dist - nei_physics_obj.speed.min(2.5) - nei_physics_obj.radius
        {
            continue;
        }

        min_front_dist = min_front_dist.min(dist - my_radius - nei_physics_obj.radius - 5.0);
    }
    min_front_dist
}
