extern crate proc_macro;

use proc_macro::TokenStream;

use quote::quote;
use syn::DeriveInput;
use syn::{parse_macro_input, Data, DataStruct, Fields};

#[proc_macro_derive(Interpolate)]
pub fn derive_answer_fn(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, data, .. } = parse_macro_input!(input);

    let body = match data {
        Data::Struct(DataStruct {
            fields: Fields::Named(fields),
            ..
        }) => {
            let field_name = fields.named.iter().map(|field| &field.ident);
            quote! {
                Self {
                    #(
                        #field_name: self.#field_name.lerp(other.value, t),
                    )*
                }
            }
        }
        Data::Struct(DataStruct {
            fields: Fields::Unnamed(_),
            ..
        }) => quote! { Self(self.0.lerp(other.0, t)) },
        _ => panic!("expected a struct"),
    };
    let output = quote! {
        impl ::bevy_replicon_snap::Interpolate for #ident {
            fn interpolate(&self, other: Self, t: f32) -> Self {
              #body
            }
        }
    };
    output.into()
}

#[proc_macro_derive(SnapSerialize)]
pub fn derive_snap_serialize(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);
    let output = quote! {
        impl ::bevy_replicon_snap::SnapSerialize for #ident {
            fn snap_serialize(
                component: ::bevy::ptr::Ptr,
                mut cursor: &mut ::std::io::Cursor<Vec<u8>>,
            ) -> Result<(), ::bevy_replicon::bincode::Error> {
                // SAFETY: Function called for registered `ComponentId`.
                let component: &#ident = unsafe { component.deref() };
                ::bevy_replicon::bincode::serialize_into(cursor, &component)
            }
        }
    };
    output.into()
}

#[proc_macro_derive(SnapDeserialize)]
pub fn derive_snap_deserialize(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);
    let output = quote! {
        impl ::bevy_replicon_snap::SnapDeserialize for #ident {
            fn snap_deserialize(
                entity: &mut ::bevy::ecs::world::EntityMut,
                _entity_map: &mut ::bevy_replicon::prelude::ServerEntityMap,
                mut cursor: &mut ::std::io::Cursor<::bevy_replicon::renet::Bytes>,
                tick: ::bevy_replicon::prelude::RepliconTick,
            ) -> Result<(), ::bevy_replicon::bincode::Error> {
                let component: #ident = ::bevy_replicon::bincode::deserialize_from(&mut cursor)?;
                if let Some(mut buffer) = entity.get_mut::<::bevy_replicon_snap::SnapshotBuffer<#ident>>() {
                    buffer.insert(component, tick.get());
                } else {
                    entity.insert(component);
                }
                Ok(())
            }
        }
    };
    output.into()
}
