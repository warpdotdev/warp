//! Proc macro for `#[derive(SettingsValue)]`.
//!
//! Generates `SettingsValue` implementations:
//! - **Enums**: variant names are converted to snake_case. Data-carrying
//!   variants recursively call `to_file_value` on their inner data.
//! - **Structs**: each field is serialized/deserialized by recursively calling
//!   the trait methods. Field names use the Rust identifier (already snake_case)
//!   unless overridden by `#[serde(rename = "...")]`.
//!   Newtype structs (`struct Foo(T)`) delegate to the inner type.

extern crate proc_macro;

use convert_case::{Case, Casing};
use proc_macro::TokenStream;
use quote::quote;
use syn::{Data, DeriveInput, Fields, Lit, Meta, MetaNameValue, parse_macro_input};

/// Derive macro that generates a `SettingsValue` implementation.
///
/// # Enums
///
/// Unit variants are serialized as snake_case JSON strings.  Data-carrying
/// variants (tuple or struct) are serialized as a single-key JSON object
/// `{ "snake_case_variant": <recursive value> }`.
///
/// # Structs
///
/// Named-field structs are serialized into a JSON object where each field value
/// is produced by calling `to_file_value()` recursively.  Field names default to
/// the Rust field name but respect `#[serde(rename = "...")]`.
///
/// Newtype structs (`struct Foo(T)`) delegate to the inner type's
/// `SettingsValue` impl.
///
/// Fields marked `#[serde(skip)]` are excluded from serialization and
/// populated via `Default` during deserialization.  Fields with
/// `#[serde(default)]` fall back to `Default` when absent in the JSON.
#[proc_macro_derive(SettingsValue, attributes(serde))]
pub fn derive_settings_value(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;

    // Read container-level #[serde(rename_all = "...")] if present.
    let container_rename_all = get_serde_rename_all(&input.attrs);

    let (impl_generics, ty_generics, where_clause) = input.generics.split_for_impl();

    let expanded = match &input.data {
        Data::Enum(data_enum) => {
            let to_arms = data_enum.variants.iter().map(|variant| {
                let variant_ident = &variant.ident;
                let cfg_attrs = get_cfg_attrs(&variant.attrs);
                let file_name = file_variant_name(variant_ident, &variant.attrs, container_rename_all.as_deref());

                match &variant.fields {
                    Fields::Unit => {
                        quote! {
                            #(#cfg_attrs)*
                            #name::#variant_ident => {
                                serde_json::Value::String(#file_name.to_string())
                            }
                        }
                    }
                    Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                        quote! {
                            #(#cfg_attrs)*
                            #name::#variant_ident(inner) => {
                                let mut obj = serde_json::Map::new();
                                obj.insert(
                                    #file_name.to_string(),
                                    ::settings_value::SettingsValue::to_file_value(inner),
                                );
                                serde_json::Value::Object(obj)
                            }
                        }
                    }
                    Fields::Unnamed(fields) => {
                        let field_bindings: Vec<_> = (0..fields.unnamed.len())
                            .map(|i| quote::format_ident!("f{i}"))
                            .collect();
                        quote! {
                            #(#cfg_attrs)*
                            #name::#variant_ident(#(#field_bindings),*) => {
                                let arr = serde_json::Value::Array(vec![
                                    #(::settings_value::SettingsValue::to_file_value(#field_bindings)),*
                                ]);
                                let mut obj = serde_json::Map::new();
                                obj.insert(#file_name.to_string(), arr);
                                serde_json::Value::Object(obj)
                            }
                        }
                    }
                    Fields::Named(fields) => {
                        let field_idents: Vec<_> = fields.named.iter()
                            .map(|f| f.ident.as_ref().unwrap())
                            .collect();
                        let field_keys: Vec<_> = fields.named.iter()
                            .map(|f| {
                                let ident = f.ident.as_ref().unwrap();
                                get_serde_rename(&f.attrs).unwrap_or_else(|| ident.to_string())
                            })
                            .collect();
                        quote! {
                            #(#cfg_attrs)*
                            #name::#variant_ident { #(#field_idents),* } => {
                                let mut inner_obj = serde_json::Map::new();
                                #(
                                    inner_obj.insert(
                                        #field_keys.to_string(),
                                        ::settings_value::SettingsValue::to_file_value(#field_idents),
                                    );
                                )*
                                let mut obj = serde_json::Map::new();
                                obj.insert(#file_name.to_string(), serde_json::Value::Object(inner_obj));
                                serde_json::Value::Object(obj)
                            }
                        }
                    }
                }
            });

            let from_arms = data_enum.variants.iter().map(|variant| {
                let variant_ident = &variant.ident;
                let cfg_attrs = get_cfg_attrs(&variant.attrs);
                let file_name = file_variant_name(variant_ident, &variant.attrs, container_rename_all.as_deref());

                match &variant.fields {
                    Fields::Unit => {
                        quote! {
                            #(#cfg_attrs)*
                            serde_json::Value::String(s) if s == #file_name => {
                                Some(#name::#variant_ident)
                            }
                        }
                    }
                    Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                        let ty = &fields.unnamed.first().unwrap().ty;
                        quote! {
                            #(#cfg_attrs)*
                            serde_json::Value::Object(obj) if obj.contains_key(#file_name) => {
                                let inner_val = obj.get(#file_name)?;
                                let inner = <#ty as ::settings_value::SettingsValue>::from_file_value(inner_val)?;
                                Some(#name::#variant_ident(inner))
                            }
                        }
                    }
                    Fields::Unnamed(fields) => {
                        let field_types: Vec<_> = fields.unnamed.iter()
                            .map(|f| &f.ty)
                            .collect();
                        let field_indices: Vec<_> = (0..fields.unnamed.len())
                            .map(syn::Index::from)
                            .collect();
                        quote! {
                            #(#cfg_attrs)*
                            serde_json::Value::Object(obj) if obj.contains_key(#file_name) => {
                                let arr = obj.get(#file_name)?.as_array()?;
                                Some(#name::#variant_ident(
                                    #(
                                        <#field_types as ::settings_value::SettingsValue>::from_file_value(arr.get(#field_indices)?)?
                                    ),*
                                ))
                            }
                        }
                    }
                    Fields::Named(fields) => {
                        let field_idents: Vec<_> = fields.named.iter()
                            .map(|f| f.ident.as_ref().unwrap())
                            .collect();
                        let field_keys: Vec<_> = fields.named.iter()
                            .map(|f| {
                                let ident = f.ident.as_ref().unwrap();
                                get_serde_rename(&f.attrs).unwrap_or_else(|| ident.to_string())
                            })
                            .collect();
                        let field_types: Vec<_> = fields.named.iter()
                            .map(|f| &f.ty)
                            .collect();
                        quote! {
                            #(#cfg_attrs)*
                            serde_json::Value::Object(obj) if obj.contains_key(#file_name) => {
                                let inner_obj = obj.get(#file_name)?.as_object()?;
                                Some(#name::#variant_ident {
                                    #(
                                        #field_idents: <#field_types as ::settings_value::SettingsValue>::from_file_value(inner_obj.get(#field_keys)?)?,
                                    )*
                                })
                            }
                        }
                    }
                }
            });

            quote! {
                impl #impl_generics ::settings_value::SettingsValue for #name #ty_generics #where_clause {
                    fn to_file_value(&self) -> serde_json::Value {
                        match self {
                            #(#to_arms)*
                        }
                    }

                    fn from_file_value(value: &serde_json::Value) -> Option<Self> {
                        match value {
                            #(#from_arms)*
                            _ => None,
                        }
                    }
                }
            }
        }
        Data::Struct(data_struct) => match &data_struct.fields {
            Fields::Named(fields) => derive_named_struct(
                name,
                fields,
                &input.attrs,
                &impl_generics,
                &ty_generics,
                where_clause,
            ),
            Fields::Unnamed(fields) if fields.unnamed.len() == 1 => {
                // Newtype struct: delegate to inner type.
                let inner_ty = &fields.unnamed.first().unwrap().ty;
                quote! {
                    impl #impl_generics ::settings_value::SettingsValue for #name #ty_generics #where_clause {
                        fn to_file_value(&self) -> serde_json::Value {
                            ::settings_value::SettingsValue::to_file_value(&self.0)
                        }

                        fn from_file_value(value: &serde_json::Value) -> Option<Self> {
                            Some(Self(<#inner_ty as ::settings_value::SettingsValue>::from_file_value(value)?))
                        }
                    }
                }
            }
            _ => {
                return syn::Error::new_spanned(
                    &input.ident,
                    "SettingsValue derive only supports structs with named fields or newtype structs",
                )
                .to_compile_error()
                .into();
            }
        },
        Data::Union(_) => {
            return syn::Error::new_spanned(
                &input.ident,
                "SettingsValue cannot be derived for unions",
            )
            .to_compile_error()
            .into();
        }
    };

    expanded.into()
}

fn derive_named_struct(
    name: &syn::Ident,
    fields: &syn::FieldsNamed,
    struct_attrs: &[syn::Attribute],
    impl_generics: &syn::ImplGenerics,
    ty_generics: &syn::TypeGenerics,
    where_clause: Option<&syn::WhereClause>,
) -> proc_macro2::TokenStream {
    // When the struct has `#[serde(default)]`, all fields fall back to
    // `Self::default()` field values (matching serde behaviour).  This does
    // not require individual field types to implement Default.
    let struct_has_default = has_serde_default(struct_attrs);

    let non_skipped_fields: Vec<_> = fields
        .named
        .iter()
        .filter(|f| !has_serde_skip(&f.attrs))
        .collect();

    let skipped_fields: Vec<_> = fields
        .named
        .iter()
        .filter(|f| has_serde_skip(&f.attrs))
        .collect();

    let to_inserts = non_skipped_fields.iter().map(|f| {
        let ident = f.ident.as_ref().unwrap();
        let key = get_serde_rename(&f.attrs).unwrap_or_else(|| ident.to_string());
        quote! {
            obj.insert(
                #key.to_string(),
                ::settings_value::SettingsValue::to_file_value(&self.#ident),
            );
        }
    });

    let from_fields = non_skipped_fields.iter().map(|f| {
        let ident = f.ident.as_ref().unwrap();
        let ty = &f.ty;
        let key = get_serde_rename(&f.attrs).unwrap_or_else(|| ident.to_string());
        let field_has_default = has_serde_default(&f.attrs);

        if struct_has_default {
            // Struct has #[serde(default)]: use Self::default() when the field
            // is absent, but propagate failure (return None) when the field is
            // present but cannot be parsed.  This matches serde semantics where
            // #[serde(default)] only applies to missing fields, not to fields
            // with invalid values.
            quote! {
                #ident: match obj.get(#key) {
                    Some(v) => <#ty as ::settings_value::SettingsValue>::from_file_value(v)?,
                    None => __struct_default.#ident,
                },
            }
        } else if field_has_default {
            // Field has #[serde(default)]: same semantics — default when
            // absent, fail when present but unparsable.
            quote! {
                #ident: match obj.get(#key) {
                    Some(v) => <#ty as ::settings_value::SettingsValue>::from_file_value(v)?,
                    None => Default::default(),
                },
            }
        } else if is_option_type(ty) {
            quote! {
                #ident: match obj.get(#key) {
                    Some(v) => <#ty as ::settings_value::SettingsValue>::from_file_value(v)?,
                    None => None,
                },
            }
        } else {
            quote! {
                #ident: <#ty as ::settings_value::SettingsValue>::from_file_value(obj.get(#key)?)?,
            }
        }
    });

    let skipped_field_defaults = skipped_fields.iter().map(|f| {
        let ident = f.ident.as_ref().unwrap();
        quote! { #ident: Default::default(), }
    });

    let struct_default_binding = struct_has_default.then(|| {
        quote! { let __struct_default = <#name #ty_generics as Default>::default(); }
    });

    quote! {
        impl #impl_generics ::settings_value::SettingsValue for #name #ty_generics #where_clause {
            fn to_file_value(&self) -> serde_json::Value {
                let mut obj = serde_json::Map::new();
                #(#to_inserts)*
                serde_json::Value::Object(obj)
            }

            fn from_file_value(value: &serde_json::Value) -> Option<Self> {
                let obj = value.as_object()?;
                #struct_default_binding
                Some(Self {
                    #(#from_fields)*
                    #(#skipped_field_defaults)*
                })
            }
        }
    }
}

/// Computes the file-format name for an enum variant.
///
/// Priority: `#[serde(rename = "...")]` > container `rename_all` > snake_case
/// of the Rust variant name.
fn file_variant_name(
    ident: &syn::Ident,
    attrs: &[syn::Attribute],
    container_rename_all: Option<&str>,
) -> String {
    // Explicit per-variant rename takes priority.
    if let Some(renamed) = get_serde_rename(attrs) {
        return renamed.to_case(Case::Snake);
    }

    let base = ident.to_string();

    // Apply container rename_all first, then convert to snake_case.
    if let Some(rename_all) = container_rename_all {
        // If the container already uses snake_case or lowercase, apply that
        // directly.  Otherwise use the variant name as-is (PascalCase) and
        // convert to snake_case.
        match rename_all {
            "snake_case" => return base.to_case(Case::Snake),
            "camelCase" => return base.to_case(Case::Camel).to_case(Case::Snake),
            "SCREAMING_SNAKE_CASE" => return base.to_case(Case::UpperSnake).to_case(Case::Snake),
            "lowercase" => return base.to_lowercase(),
            _ => {}
        }
    }

    // Default: PascalCase → snake_case
    base.to_case(Case::Snake)
}

/// Collects all `#[cfg(...)]` attributes so they can be propagated onto
/// generated match arms.
fn get_cfg_attrs(attrs: &[syn::Attribute]) -> Vec<&syn::Attribute> {
    attrs.iter().filter(|a| a.path().is_ident("cfg")).collect()
}

/// Reads `#[serde(rename = "...")]` from field/variant attributes.
fn get_serde_rename(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        if let Ok(nested) = attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            for meta in &nested {
                if let Meta::NameValue(MetaNameValue {
                    path,
                    value: syn::Expr::Lit(expr_lit),
                    ..
                }) = meta
                    && path.is_ident("rename")
                    && let Lit::Str(s) = &expr_lit.lit
                {
                    return Some(s.value());
                }
            }
        }
    }
    None
}

/// Reads `#[serde(rename_all = "...")]` from container attributes.
fn get_serde_rename_all(attrs: &[syn::Attribute]) -> Option<String> {
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        if let Ok(nested) = attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            for meta in &nested {
                if let Meta::NameValue(MetaNameValue {
                    path,
                    value: syn::Expr::Lit(expr_lit),
                    ..
                }) = meta
                    && path.is_ident("rename_all")
                    && let Lit::Str(s) = &expr_lit.lit
                {
                    return Some(s.value());
                }
            }
        }
    }
    None
}

/// Returns `true` if the field has `#[serde(skip)]`.
fn has_serde_skip(attrs: &[syn::Attribute]) -> bool {
    has_serde_flag(attrs, "skip")
}

/// Returns `true` if the field has `#[serde(default)]`.
fn has_serde_default(attrs: &[syn::Attribute]) -> bool {
    has_serde_flag(attrs, "default")
}

/// Returns `true` if the type looks like `Option<...>`.
///
/// This is a best-effort heuristic — it checks whether the last segment of
/// the type path is `Option`.  It won't detect type aliases but covers the
/// vast majority of real-world usage.
fn is_option_type(ty: &syn::Type) -> bool {
    if let syn::Type::Path(type_path) = ty
        && let Some(segment) = type_path.path.segments.last()
    {
        return segment.ident == "Option";
    }
    false
}

/// Returns `true` if any `#[serde(...)]` attribute contains the given flag.
fn has_serde_flag(attrs: &[syn::Attribute], flag: &str) -> bool {
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        if let Ok(nested) = attr
            .parse_args_with(syn::punctuated::Punctuated::<Meta, syn::Token![,]>::parse_terminated)
        {
            for meta in &nested {
                if let Meta::Path(path) = meta
                    && path.is_ident(flag)
                {
                    return true;
                }
            }
        }
    }
    false
}
