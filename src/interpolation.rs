use std::{collections::VecDeque, io::Cursor};

use bevy::{
    app::{App, PreUpdate},
    ecs::{
        component::Component,
        entity::Entity,
        query::{Added, Or, With, Without},
        schedule::{common_conditions::resource_exists, IntoSystemConfigs},
        system::{Commands, Query, Res, Resource},
        world::EntityMut,
    },
    prelude::Transform,
    reflect::Reflect,
    time::Time,
    utils::default,
};
use bevy_replicon::{
    bincode,
    core::{
        command_markers::{AppMarkerExt, MarkerConfig},
        common_conditions::client_connected,
        ctx::{RemoveCtx, WriteCtx},
        replication_registry::rule_fns::RuleFns,
        replication_rules::AppRuleExt,
    },
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::{
    prediction::{owner_prediction_init_system, predicted_snapshot_system, Predicted},
    InterpolationSet,
};

pub trait Interpolate {
    fn interpolate(&self, other: Self, t: f32) -> Self;
}

impl Interpolate for Transform {
    fn interpolate(&self, other: Self, t: f32) -> Self {
        let translation = self.translation.lerp(other.translation, t);
        let rotation = self.rotation.lerp(other.rotation, t);
        let scale = self.scale.lerp(other.scale, t);
        Transform {
            translation,
            rotation,
            scale,
        }
    }
}

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct Interpolated;

#[derive(Deserialize, Serialize, Reflect)]
pub struct Snapshot<T: Component + Interpolate + Clone> {
    tick: u32,
    value: T,
}

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct SnapshotBuffer<T: Component + Interpolate + Clone> {
    pub buffer: VecDeque<T>,
    pub time_since_last_snapshot: f32,
    pub latest_snapshot_tick: u32,
}

#[derive(Resource, Serialize, Deserialize, Debug)]
pub struct SnapshotInterpolationConfig {
    pub max_tick_rate: u16,
}

#[derive(Component)]
pub struct RecordSnapshotsMarker;

impl<T: Component + Interpolate + Clone> SnapshotBuffer<T> {
    pub fn new() -> Self {
        Self {
            buffer: VecDeque::new(),
            time_since_last_snapshot: 0.0,
            latest_snapshot_tick: 0,
        }
    }
    pub fn insert(&mut self, element: T, tick: u32) {
        if self.buffer.len() > 1 {
            self.buffer.pop_front();
        }
        self.buffer.push_back(element);
        self.time_since_last_snapshot = 0.0;
        self.latest_snapshot_tick = tick;
    }

    pub fn latest_snapshot(&self) -> T {
        self.buffer.iter().last().unwrap().clone()
    }

    pub fn latest_snapshot_tick(&self) -> u32 {
        self.latest_snapshot_tick
    }

    pub fn age(&self) -> f32 {
        self.time_since_last_snapshot
    }
}

/// Interpolate between snapshots.
pub fn snapshot_interpolation_system<T: Component + Interpolate + Clone>(
    mut q: Query<(&mut T, &mut SnapshotBuffer<T>), (With<Interpolated>, Without<Predicted>)>,
    time: Res<Time>,
    config: Res<SnapshotInterpolationConfig>,
) {
    for (mut component, mut snapshot_buffer) in q.iter_mut() {
        let buffer = &snapshot_buffer.buffer;
        let elapsed = snapshot_buffer.time_since_last_snapshot;
        if buffer.len() < 2 {
            continue;
        }

        let tick_duration = 1.0 / (config.max_tick_rate as f32);

        if elapsed > tick_duration + time.delta_seconds() {
            continue;
        }

        let t = (elapsed / tick_duration).clamp(0., 1.);
        *component = buffer[0].interpolate(buffer[1].clone(), t);
        snapshot_buffer.time_since_last_snapshot += time.delta_seconds();
    }
}

/// Add a marker to all components requiring a snapshot buffer
pub fn snapshot_buffer_init_system<T: Component + Interpolate + Clone>(
    q_new: Query<(Entity, &T), Or<(Added<Predicted>, Added<Interpolated>)>>,
    mut commands: Commands,
) {
    for (e, _v) in q_new.iter() {
        commands.entity(e).insert(RecordSnapshotsMarker);
    }
}

pub fn write_snap_component<C: Clone + Interpolate + Component + DeserializeOwned>(
    ctx: &mut WriteCtx,
    rule_fns: &RuleFns<C>,
    entity: &mut EntityMut,
    cursor: &mut Cursor<&[u8]>,
) -> bincode::Result<()> {
    let component: C = rule_fns.deserialize(ctx, cursor)?;
    if let Some(mut buffer) = entity.get_mut::<SnapshotBuffer<C>>() {
        buffer.insert(component, ctx.message_tick.get());
    } else {
        let mut buffer = SnapshotBuffer::new();
        buffer.insert(component, ctx.message_tick.get());
        ctx.commands.entity(entity.id()).insert(buffer);
    }

    Ok(())
}

fn remove_snap_component<C: Clone + Interpolate + Component + DeserializeOwned>(
    ctx: &mut RemoveCtx,
    entity: &mut EntityMut,
) {
    ctx.commands
        .entity(entity.id())
        .remove::<SnapshotBuffer<C>>()
        .remove::<C>();
}
pub trait AppInterpolationExt {
    /// Register a component to be replicated and interpolated between server updates
    /// Requires the component to implement the Interpolate trait
    fn replicate_interpolated<C>(&mut self) -> &mut Self
    where
        C: Component + Interpolate + Clone + Serialize + DeserializeOwned;
}

impl AppInterpolationExt for App {
    fn replicate_interpolated<T>(&mut self) -> &mut Self
    where
        T: Component + Interpolate + Clone + Serialize + DeserializeOwned,
    {
        self.add_systems(
            PreUpdate,
            (snapshot_buffer_init_system::<T>.after(owner_prediction_init_system))
                .in_set(InterpolationSet::Init)
                .run_if(client_connected),
        );
        self.add_systems(
            PreUpdate,
            (
                snapshot_interpolation_system::<T>,
                predicted_snapshot_system::<T>,
            )
                .chain()
                .in_set(InterpolationSet::Interpolate)
                .run_if(client_connected),
        )
        .replicate::<T>()
        .register_marker_with::<RecordSnapshotsMarker>(MarkerConfig {
            need_history: true,
            ..default()
        })
        .set_marker_fns::<RecordSnapshotsMarker, T>(
            write_snap_component::<T>,
            remove_snap_component::<T>,
        )
    }
}
