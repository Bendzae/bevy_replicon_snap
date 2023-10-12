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
        impl Interpolate for #ident {
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
        impl SnapSerialize for #ident {
            fn snap_serialize(
                component: Ptr,
                mut cursor: &mut Cursor<Vec<u8>>,
            ) -> Result<(), bincode::Error> {
                // SAFETY: Function called for registered `ComponentId`.
                let component: &#ident = unsafe { component.deref() };
                bincode::serialize_into(cursor, &component)
            }
        }
    };
    output.into()
}

#[proc_macro_derive(SnapDeserialize)]
pub fn derive_snap_deserialize(input: TokenStream) -> TokenStream {
    let DeriveInput { ident, .. } = parse_macro_input!(input);
    let output = quote! {
        impl SnapDeserialize for #ident {
             fn snap_deserialize(
                entity: &mut EntityMut,
                _entity_map: &mut ServerEntityMap,
                mut cursor: &mut Cursor<Bytes>,
                tick: RepliconTick,
            ) -> Result<(), bincode::Error> {
                let value: Vec2 = bincode::deserialize_from(&mut cursor)?;
                let component = #ident(value);
                if let Some(mut buffer) = entity.get_mut::<SnapshotBuffer<#ident>>() {
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
