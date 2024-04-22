use bevy::prelude::*;
use crate::Interpolate;

impl Interpolate for Transform {
    fn interpolate(&self, other: Self, t: f32) -> Self {
        self.lerp(other, t)
    }
}