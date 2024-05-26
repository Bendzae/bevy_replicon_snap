use bevy::{
    app::{App, Update},
    ecs::{
        component::Component,
        entity::Entity,
        event::{Event, EventReader},
        query::{Added, With, Without},
        schedule::{common_conditions::resource_exists, IntoSystemConfigs},
        system::{Commands, Query, Res, ResMut, Resource},
    },
    reflect::Reflect,
    time::Time,
};
use bevy_replicon::{
    client::confirmed::Confirmed,
    core::{
        common_conditions::has_authority, replication_rules::AppRuleExt,
        replicon_channels::RepliconChannel,
    },
    network_event::client_event::{ClientEventAppExt, FromClient},
};
use bevy_replicon_renet::renet::{transport::NetcodeClientTransport, RenetClient};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::collections::vec_deque::Iter;
use std::collections::VecDeque;
use std::fmt::Debug;

use crate::{
    interpolation::Interpolate, interpolation::SnapshotBuffer, Interpolated, NetworkOwner,
};

/// This trait defines how an event will mutate a given component
/// and is required for prediction.
pub trait Predict<E: Event, T>
where
    Self: Component + Interpolate,
{
    fn apply_event(&mut self, event: &E, delta_time: f32, context: &T);
}

pub struct EventSnapshot<T: Event> {
    pub value: T,
    pub tick: u32,
    pub delta_time: f32,
}

#[derive(Resource)]
pub struct PredictedEventHistory<T: Event>(pub VecDeque<EventSnapshot<T>>);

#[derive(Component, Deserialize, Serialize, Reflect)]
pub struct OwnerPredicted;

#[derive(Component, Reflect)]
pub struct Predicted;

impl<T: Event> PredictedEventHistory<T> {
    pub fn new() -> PredictedEventHistory<T> {
        Self(VecDeque::new())
    }
    pub fn insert(&mut self, value: T, tick: u32, delta_time: f32) -> &mut Self {
        self.0.push_back(EventSnapshot {
            value,
            tick,
            delta_time,
        });
        self
    }
    pub fn remove_stale(&mut self, latest_server_snapshot_tick: u32) -> &mut Self {
        if let Some(last_index) = self
            .0
            .iter()
            .position(|v| v.tick >= latest_server_snapshot_tick)
        {
            self.0.drain(0..last_index);
        } else {
            self.0.clear();
        }
        self
    }

    pub fn predict(&mut self, latest_server_snapshot_tick: u32) -> Iter<'_, EventSnapshot<T>> {
        self.remove_stale(latest_server_snapshot_tick);
        self.0.iter()
    }
}

pub fn owner_prediction_init_system(
    q_owners: Query<(Entity, &NetworkOwner), Added<OwnerPredicted>>,
    client: Res<NetcodeClientTransport>,
    mut commands: Commands,
) {
    let client_id = client.client_id();
    for (e, id) in q_owners.iter() {
        if id.0 == client_id.raw() {
            commands.entity(e).insert(Predicted);
        } else {
            commands.entity(e).insert(Interpolated);
        }
    }
}

/// Advances the snapshot buffer time for predicted entities.
pub fn predicted_snapshot_system<T: Component + Interpolate + Clone>(
    mut q: Query<&mut SnapshotBuffer<T>, (Without<Interpolated>, With<Predicted>)>,
    time: Res<Time>,
) {
    for mut snapshot_buffer in q.iter_mut() {
        snapshot_buffer.time_since_last_snapshot += time.delta_seconds();
    }
}

/// Server implementation
pub fn server_update_system<
    E: Event,
    T: Component,
    C: Component + Interpolate + Predict<E, T> + Clone,
>(
    time: Res<Time>,
    mut move_events: EventReader<FromClient<E>>,
    mut subjects: Query<(&NetworkOwner, &mut C, &T), Without<Predicted>>,
) {
    for FromClient { client_id, event } in move_events.read() {
        for (player, mut component, context) in &mut subjects {
            if client_id.get() == player.0 {
                component.apply_event(event, time.delta_seconds(), context);
            }
        }
    }
}

// Client prediction implementation
pub fn predicted_update_system<
    E: Event + Clone,
    T: Component,
    C: Component + Interpolate + Predict<E, T> + Clone,
>(
    mut q_predicted_players: Query<
        (&mut C, &SnapshotBuffer<C>, &Confirmed, &T),
        (With<Predicted>, Without<Interpolated>),
    >,
    mut local_events: EventReader<E>,
    mut event_history: ResMut<PredictedEventHistory<E>>,
    time: Res<Time>,
) {
    // Apply all pending inputs to latest snapshot
    for (mut component, snapshot_buffer, confirmed, context) in q_predicted_players.iter_mut() {
        // Append the latest input event
        for event in local_events.read() {
            event_history.insert(
                event.clone(),
                confirmed.last_tick().get(),
                time.delta_seconds(),
            );
        }

        let mut corrected_component = snapshot_buffer.latest_snapshot();
        for event_snapshot in event_history.predict(snapshot_buffer.latest_snapshot_tick()) {
            corrected_component.apply_event(
                &event_snapshot.value,
                event_snapshot.delta_time,
                context,
            );
        }
        *component = corrected_component;
    }
}

pub trait AppPredictionExt {
    /// Register an event for client-side prediction, this will make sure a history of past events
    /// is stored for the client to be able to replay them in case of a server correction
    fn add_client_predicted_event<E>(&mut self, channel: impl Into<RepliconChannel>) -> &mut Self
    where
        E: Event + Serialize + DeserializeOwned + Debug + Clone;

    /// Register a component and event pair for prediction.
    /// This will generate serverside and clientside systems that use the implementation from the
    /// `Predict` trait to allow prediction and serverside correction
    fn predict_event_for_component<E, T, C>(&mut self) -> &mut Self
    where
        E: Event + Serialize + DeserializeOwned + Debug + Clone,
        T: Component + Serialize + DeserializeOwned,
        C: Component + Predict<E, T> + Clone;
}

impl AppPredictionExt for App {
    fn add_client_predicted_event<E>(&mut self, channel: impl Into<RepliconChannel>) -> &mut Self
    where
        E: Event + Serialize + DeserializeOwned + Debug + Clone,
    {
        let history: PredictedEventHistory<E> = PredictedEventHistory::new();
        self.insert_resource(history);
        self.add_client_event::<E>(channel)
    }

    fn predict_event_for_component<E, T, C>(&mut self) -> &mut Self
    where
        E: Event + Serialize + DeserializeOwned + Debug + Clone,
        T: Component + Serialize + DeserializeOwned,
        C: Component + Predict<E, T> + Clone,
    {
        self.add_systems(
            Update,
            (
                server_update_system::<E, T, C>.run_if(has_authority), // Runs only on the server or a single player.
                predicted_update_system::<E, T, C>.run_if(resource_exists::<RenetClient>), // Runs only on clients.
            ),
        )
        .replicate::<T>()
    }
}
