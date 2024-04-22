use bevy::prelude::*;
use crate::Interpolate;

impl Interpolate for Transform {
    fn interpolate(&self, other: Self, t: f32) -> Self {
        let mut to_return = self.clone();

        to_return.translation = self.translation.lerp(other.translation, t);
        to_return.rotation = self.rotation.slerp(other.rotation, t);
        to_return.scale = self.scale.lerp(other.scale, t);

        to_return
    }
}