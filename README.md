[![CI](https://github.com/Bendzae/bevy_replicon_snap/actions/workflows/rust.yml/badge.svg)](https://github.com/Bendzae/bevy_replicon_snap/actions/workflows/rust.yml)
[![crates.io](https://img.shields.io/crates/v/bevy_replicon_snap.svg)](https://crates.io/crates/bevy_replicon_snap)

# bevy_replicon_snap

A
[Snapshot Interpolation](https://www.snapnet.dev/blog/netcode-architectures-part-3-snapshot-interpolation/)
plugin for the networking solution
[bevy_replicon](https://github.com/lifescapegame/bevy_replicon/tree/master) in
the [Bevy](https://github.com/bevyengine/bevy/tree/main) game engine.

_**This library is a very rough proof of concept and not meant to be used in
productive games**_

## Features

- Basic but customizable snapshot interpolation for replicated components
- Client-Side prediction:
  - Owner predicted: Owner client of the entity predicts, other clients
    interpolate

In the
[examples](https://github.com/Bendzae/bevy_replicon_snap/tree/main/examples) you
can find a clone of the `Simple Box` example of `bevy_replicon`, in 3 versions:
no interpolation or prediction, interpolated, predicted. I recommend to look at
the diffs between those examples to gain a better understanding how this plugin
works.

## Usage

### Setup

Add the bevy_replicon plugin and this plugin to your bevy application.

The plugin needs to know the maximum server tick rate to estimate time between
snapshots so it needs to be passed in on initialization:

```rust
const MAX_TICK_RATE: u16 = 30;

...

.add_plugins((
    DefaultPlugins,
    RepliconPlugins.build().set(ServerPlugin {
        tick_policy: TickPolicy::MaxTickRate(MAX_TICK_RATE),
        ..default()
    }),
    RepliconRenetPlugins,
    SnapshotInterpolationPlugin {
        max_tick_rate: MAX_TICK_RATE,
    },
))

...
```

### Interpolation

To allow a Component to be interpolated it needs to implement the traits:
`Interpolate`, `Serialze` and `Deserialize`.

This lib provides a basic derive macro for `Interpolate` but for complex types
you will have to implement it yourself.

```rust
use bevy_replicon_snap_macros::{Interpolate};

#[derive(Component, Deserialize, Serialize, Interpolate, Clone)] 
struct PlayerPosition(Vec2);
```

Next you need to register the component for Interpolation:

```rust
app.replicate_interpolated::<PlayerPosition>()
```

this also registers the component for replication by bevy_replicon.

Last Step is to add the `Interpolated` Component to any entity that should be
interpolated.

```rust
commands.spawn((
    PlayerPosition(Vec2::ZERO),
    Replication,
    Interpolated,
    ...
));
```

### Client-Side Prediction

Coming soon.. In the meantime check the "predicted" example!

## Alternatives

- [bevy_timewarp](https://github.com/RJ/bevy_timewarp) An awesome
  predict/rollback library that also has integration with bevy_replicon
