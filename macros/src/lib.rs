extern crate proc_macro;

use proc_macro::TokenStream;

use quote::quote;
use syn::DeriveInput;
use syn::{parse_macro_input, Data, DataStruct, Fields};

#[proc_macro_derive(Interpolate)]
pub fn derive_interpolate(input: TokenStream) -> TokenStream {
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
        impl bevy_replicon_snap::interpolation::Interpolate for #ident {
            fn interpolate(&self, other: Self, t: f32) -> Self {
              #body
            }
        }
    };
    output.into()
}
