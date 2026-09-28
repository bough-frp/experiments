//! `#[derive(Trace)]`, and the check that a `#[rebrand(skip)]` field holds no
//! token, behind the `trace` feature, for `rfd-0003-brand-erasure -- trace`.
//! The trait is the api's:
//!
//! ```text
//! pub trait Trace {
//!     fn trace(&self, tracer: &mut Tracer);
//! }
//! ```
//!
//! What it writes, for `struct Pair<'g, T> { cell: Cell<'g, u32>, value: T,
//! #[rebrand(skip)] unit: Celsius }`:
//!
//! ```text
//! impl<'g, T: ::bough::Trace> ::bough::Trace for Pair<'g, T> {
//!     fn trace(&self, tracer: &mut ::bough::Tracer) {
//!         match self {
//!             Pair { cell: f0, value: f1, unit: _ } => {
//!                 <Cell<'g, u32> as Trace>::trace(f0, tracer);
//!                 <T as Trace>::trace(f1, tracer);
//!             }
//!         }
//!     }
//! }
//! ```
//!
//! A skipped field is not traced, as it is not rebranded: it is cloned
//! across brands as it is, and the collector never sees into it. That is
//! sound only if it holds no token, so both derives check each skipped field:
//!
//! - **A type that names the type's lifetime or a type parameter** is
//!   refused with a message, before any impl is written: it may be, or hold,
//!   a token, and the derive can't see which.
//! - **Any other type is concrete**, and the check asks the compiler whether
//!   it implements `Rebrand` or `Trace`, through an inherent associated const
//!   that exists only when the bound holds and a trait const that stands in
//!   when it doesn't (the `impls` crate's trick, stable). If either holds, an
//!   `assert!` in a `const` fails, with a message naming the field. Every
//!   token implements both, and so does every container of one Bough knows;
//!   a type Bough can't see into implements neither, and that type is what
//!   `skip` is for. A skipped `String` is refused too: it rebrands through
//!   its own impl, so the skip does nothing but hide it.
//!
//! What the check can't see: a type that holds a `'static` token and
//! implements neither trait (a hand-written struct, or another crate's), as
//! `Leaf<T>` can't see into its `T`. Getting a `'static` token at all takes
//! the stash of question 3, so the two answers go together.
use proc_macro2::{TokenStream as Tokens, TokenTree};
use quote::{ToTokens, format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Error, Fields, Ident, Path, Result, parse_quote};

use super::skipped;

pub(crate) fn expand(input: &DeriveInput) -> Result<Tokens> {
    let name = &input.ident;
    let checks = skip_checks(input)?;
    let mut generics = input.generics.clone();
    for param in generics.type_params_mut() {
        param.bounds.push(parse_quote!(::bough::Trace));
    }
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    let body = match &input.data {
        Data::Struct(data) => {
            let arm = arm(&parse_quote!(#name), &data.fields)?;
            quote!(match self { #arm })
        }
        Data::Enum(data) => {
            let mut arms = Vec::new();
            for variant in &data.variants {
                let v = &variant.ident;
                arms.push(arm(&parse_quote!(#name::#v), &variant.fields)?);
            }
            let star = data.variants.is_empty().then(|| quote!(*));
            quote!(match #star self { #(#arms)* })
        }
        Data::Union(u) => {
            return Err(Error::new(
                u.union_token.span(),
                "Trace can't be derived for a union: the derive can't tell which field is live",
            ));
        }
    };

    Ok(quote! {
        impl #impl_generics ::bough::Trace for #name #ty_generics #where_clause {
            fn trace(&self, tracer: &mut ::bough::Tracer) {
                #body
            }
        }
        #checks
    })
}

/// One struct or variant: its pattern, with skipped fields bound to `_`, and
/// a call to each other field's `trace`.
fn arm(path: &Path, fields: &Fields) -> Result<Tokens> {
    let mut binds = Vec::new();
    let mut calls = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        if skipped(field)? {
            binds.push(quote!(_));
            continue;
        }
        let bind = format_ident!("__f{i}");
        let ty = &field.ty;
        // Spanned at the field's type, so a missing `Trace` is reported there.
        calls.push(quote_spanned!(ty.span()=> <#ty as ::bough::Trace>::trace(#bind, tracer);));
        binds.push(quote!(#bind));
    }
    let pattern = match fields {
        Fields::Named(named) => {
            let names = named.named.iter().map(|f| &f.ident);
            quote!(#path { #(#names: #binds),* })
        }
        Fields::Unnamed(_) => quote!(#path(#(#binds),*)),
        Fields::Unit => quote!(#path),
    };
    Ok(quote!(#pattern => { #(#calls)* }))
}

/// The check both derives emit for the skipped fields: an error for one whose
/// type names a lifetime or type parameter of the type, and a `const` of
/// assertions for the rest. Empty when nothing is skipped.
pub(crate) fn skip_checks(input: &DeriveInput) -> Result<Tokens> {
    let lifetimes: Vec<Ident> = input
        .generics
        .lifetimes()
        .map(|l| l.lifetime.ident.clone())
        .collect();
    let params: Vec<Ident> = input
        .generics
        .type_params()
        .map(|t| t.ident.clone())
        .collect();

    let mut fields = Vec::new();
    match &input.data {
        Data::Struct(data) => fields.extend(labelled("", &data.fields)),
        Data::Enum(data) => {
            for variant in &data.variants {
                fields.extend(labelled(&format!("{}::", variant.ident), &variant.fields));
            }
        }
        Data::Union(_) => {}
    }

    let mut asserts = Vec::new();
    for (label, field) in fields {
        if !skipped(field)? {
            continue;
        }
        let ty = &field.ty;
        if let Some(named) = names(ty.to_token_stream(), &lifetimes, &params) {
            return Err(Error::new(
                ty.span(),
                format!(
                    "#[rebrand(skip)] on {label}: its type names {named}, so it may hold a \
                     token, and a skipped token is hidden from the collector and from \
                     rebranding"
                ),
            ));
        }
        let rebrand = format!(
            "#[rebrand(skip)] on {label}: its type implements Rebrand, so it may hold a \
             token, and a skipped token is hidden from the collector and from rebranding; \
             drop the skip, and the field rebrands through its own impl"
        );
        let trace = format!(
            "#[rebrand(skip)] on {label}: its type implements Trace, so it may hold a \
             token, and a skipped token is hidden from the collector and from rebranding"
        );
        asserts.push(quote_spanned! {ty.span()=>
            ::core::assert!(!<__Skipped<#ty>>::REBRAND, #rebrand);
            ::core::assert!(!<__Skipped<#ty>>::TRACE, #trace);
        });
    }
    if asserts.is_empty() {
        return Ok(Tokens::new());
    }
    // `<__Skipped<F>>::REBRAND` is the inherent const when `F: Rebrand`, and
    // the trait's `false` otherwise: an inherent item whose bound fails is
    // passed over. It works for a concrete `F` only, which every type that
    // reaches here is.
    Ok(quote! {
        const _: () = {
            struct __Skipped<T: ?::core::marker::Sized>(::core::marker::PhantomData<T>);
            trait __NotRebrand {
                const REBRAND: bool = false;
            }
            trait __NotTrace {
                const TRACE: bool = false;
            }
            impl<T: ?::core::marker::Sized> __NotRebrand for __Skipped<T> {}
            impl<T: ?::core::marker::Sized> __NotTrace for __Skipped<T> {}
            impl<T: ::bough::Rebrand> __Skipped<T> {
                const REBRAND: bool = true;
            }
            impl<T: ?::core::marker::Sized + ::bough::Trace> __Skipped<T> {
                const TRACE: bool = true;
            }
            #(#asserts)*
        };
    })
}

/// Each field with the name a message calls it by: `count`, `0`,
/// `Chat::0`.
fn labelled<'a>(prefix: &str, fields: &'a Fields) -> Vec<(String, &'a syn::Field)> {
    fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            let name = f
                .ident
                .as_ref()
                .map_or_else(|| i.to_string(), ToString::to_string);
            (format!("`{prefix}{name}`"), f)
        })
        .collect()
}

/// Whether a type names one of the lifetimes or type parameters, by its
/// tokens: `'g` is a `'` then an ident.
fn names(tokens: Tokens, lifetimes: &[Ident], params: &[Ident]) -> Option<String> {
    let mut tick = false;
    for token in tokens {
        match token {
            TokenTree::Group(group) => {
                if let Some(found) = names(group.stream(), lifetimes, params) {
                    return Some(found);
                }
            }
            TokenTree::Punct(p) if p.as_char() == '\'' => {
                tick = true;
                continue;
            }
            TokenTree::Ident(ident) if tick && lifetimes.contains(&ident) => {
                return Some(format!("the lifetime `'{ident}`"));
            }
            TokenTree::Ident(ident) if !tick && params.contains(&ident) => {
                return Some(format!("the type parameter `{ident}`"));
            }
            _ => {}
        }
        tick = false;
    }
    None
}
