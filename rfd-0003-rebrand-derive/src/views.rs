//! The borrowed views, behind the `views` feature: `Borrow`, a read view of
//! the stored copy, and `BorrowMut`, a write view of it, for
//! `rfd-0003-brand-erasure -- borrow`. The traits are in that mode's
//! `borrow/api.rs`:
//!
//! ```text
//! pub trait Borrow: Rebrand {
//!     type Ref<'a>;
//!     fn borrow<'a>(from: &'a Self::Of<'static>, at: Loan<'a>) -> Self::Ref<'a>;
//! }
//! pub trait BorrowMut: Borrow {
//!     type Mut<'a>;
//!     fn borrow_mut<'a>(from: &'a mut Self::Of<'static>, at: Loan<'a>) -> Self::Mut<'a>;
//! }
//! ```
//!
//! What it writes, for `struct Pair<'g, T> { cell: Cell<'g, u32>, value: T }`:
//!
//! ```text
//! pub struct PairRef<'a, 'g, T: Borrow> {
//!     cell: <Cell<'g, u32> as Borrow>::Ref<'a>,   // Cell<'g, u32>, a copy of the id
//!     value: <T as Borrow>::Ref<'a>,
//! }
//! impl<'g, T: Rebrand + Borrow> Borrow for Pair<'g, T> {
//!     type Ref<'a> = PairRef<'a, 'g, T>;
//!     fn borrow<'a>(from: &'a Self::Of<'static>, at: Loan<'a>) -> PairRef<'a, 'g, T> {
//!         match from {
//!             Pair { cell: f0, value: f1 } => PairRef {
//!                 cell: <Cell<'g, u32> as Borrow>::borrow(f0, at),
//!                 value: <T as Borrow>::borrow(f1, at),
//!             },
//!         }
//!     }
//! }
//! ```
//!
//! and `PairMut` the same way, from `&'a mut` and `BorrowMut`. The choices:
//!
//! - **Every field's view is its type's own**, `<F as Borrow>::Ref<'a>`, so
//!   the derive needs to know nothing about tokens or containers: a token's
//!   is the token at the reader's brand, a `Vec`'s is a `ListRef`, a
//!   `String`'s is `&'a String`. A field whose type has no view is an error
//!   at that field, where the view type names its view. Only type parameters
//!   are bound: a `where F: Borrow` per field stops rustc normalizing the
//!   field's `F::Of<'static>` (it prefers the clause to the impl).
//! - **Both views for every type**, so every field needs `Borrow` and
//!   `BorrowMut`; a hand-written leaf writes both.
//! - **A type with no brand and no type parameter is its own stored copy**,
//!   so its views are `&'a Self` and `&'a mut Self`, no new type: such a
//!   type keeps the whole of the `&A` and `&mut S` API.
//! - **An enum's views are enums** of the same variants over the fields'
//!   views, so a reader matches on them as on the value.
//! - **A `Ref` is `Copy` when every field's is**, as `&A` is, so it can be
//!   passed on and read twice.
//! - **A skipped field** is `&'a F` and `&'a mut F`: it is the same type in
//!   the stored copy.
//! - **The generated types have no `Debug`**; nothing a derive on the
//!   original says carries over to them.
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{Data, DeriveInput, Fields, Generics, Lifetime, Result, WherePredicate, parse_quote};

use super::skipped;

/// Which view: the read one or the write one.
#[derive(Clone, Copy)]
enum Kind {
    Ref,
    Mut,
}

impl Kind {
    fn trait_path(self) -> Tokens {
        match self {
            Kind::Ref => quote!(::bough::Borrow),
            Kind::Mut => quote!(::bough::BorrowMut),
        }
    }

    fn assoc(self) -> Tokens {
        match self {
            Kind::Ref => quote!(Ref),
            Kind::Mut => quote!(Mut),
        }
    }

    fn method(self) -> Tokens {
        match self {
            Kind::Ref => quote!(borrow),
            Kind::Mut => quote!(borrow_mut),
        }
    }

    /// `&'a` or `&'a mut`.
    fn reference(self, a: &Lifetime) -> Tokens {
        match self {
            Kind::Ref => quote!(&#a),
            Kind::Mut => quote!(&#a mut),
        }
    }
}

/// `generics` is the `Rebrand` impl's: every type parameter bound by
/// `Rebrand`, and the `Of` bounds in its `where` clause.
pub(crate) fn expand(
    input: &DeriveInput,
    generics: &Generics,
    brand: Option<&Lifetime>,
    has_type_param: bool,
) -> Result<Tokens> {
    // The view's lifetime, distinct from the brand.
    let a = match brand {
        Some(b) if b.ident == "a" => Lifetime::new("'b", Span::call_site()),
        _ => Lifetime::new("'a", Span::call_site()),
    };
    if brand.is_none() && !has_type_param {
        return Ok(own_copy(input, generics, &a));
    }
    let mut out = Tokens::new();
    for kind in [Kind::Ref, Kind::Mut] {
        out.extend(view(input, generics, &a, kind)?);
    }
    Ok(out)
}

/// A type with no brand and no parameter: `Of<'static>` is the type itself,
/// so a view is a plain reference to it.
fn own_copy(input: &DeriveInput, generics: &Generics, a: &Lifetime) -> Tokens {
    let name = &input.ident;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        impl #impl_generics ::bough::Borrow for #name #ty_generics #where_clause {
            type Ref<#a> = &#a Self;
            fn borrow<#a>(from: &#a Self::Of<'static>, _: ::bough::Loan<#a>) -> Self::Ref<#a> {
                from
            }
        }

        impl #impl_generics ::bough::BorrowMut for #name #ty_generics #where_clause {
            type Mut<#a> = &#a mut Self;
            fn borrow_mut<#a>(from: &#a mut Self::Of<'static>, _: ::bough::Loan<#a>) -> Self::Mut<#a> {
                from
            }
        }
    }
}

/// One view: the type, and the impl that builds it.
fn view(input: &DeriveInput, generics: &Generics, a: &Lifetime, kind: Kind) -> Result<Tokens> {
    let name = &input.ident;
    let vis = &input.vis;
    let view_name = format_ident!("{}{}", name, kind.assoc().to_string());
    let (trait_path, assoc, method) = (kind.trait_path(), kind.assoc(), kind.method());
    let reference = kind.reference(a);

    // Only the type parameters are bound, `T: Borrow`. A `where F: Borrow`
    // per field would name the view's existence at the field, but rustc then
    // prefers that clause to the impl when it projects `F::Of<'static>`, and
    // the stored field's type no longer normalizes. A field whose type has no
    // view is reported where the view type names `<F as Borrow>::Ref<'a>`.
    let mut view_generics = input.generics.clone();
    for param in view_generics.type_params_mut() {
        param.bounds.push(parse_quote!(#trait_path));
    }
    view_generics.params.insert(0, parse_quote!(#a));
    let (view_impl_generics, view_ty_generics, view_where) = view_generics.split_for_impl();

    // The impl's generics: the `Rebrand` impl's, with every type parameter
    // bound by the view's trait too.
    let mut impl_generics = generics.clone();
    for param in impl_generics.type_params_mut() {
        param.bounds.push(parse_quote!(#trait_path));
    }
    let (impl_impl_generics, _, impl_where) = impl_generics.split_for_impl();
    let (_, ty_generics, _) = input.generics.split_for_impl();

    let field_view = |ty: &syn::Type, skip: bool| -> Tokens {
        if skip {
            quote!(#reference #ty)
        } else {
            quote_spanned!(ty.span()=> <#ty as #trait_path>::#assoc<#a>)
        }
    };
    let field_build = |ty: &syn::Type, skip: bool, bind: &syn::Ident| -> Tokens {
        if skip {
            quote!(#bind)
        } else {
            quote_spanned!(ty.span()=> <#ty as #trait_path>::#method(#bind, at))
        }
    };

    let (definition, body, copy_bounds) = match &input.data {
        Data::Struct(data) => {
            let (decl, pattern, built) = shape(
                &data.fields,
                &parse_quote!(#name),
                &parse_quote!(#view_name),
                &field_view,
                &field_build,
            )?;
            let definition = match &data.fields {
                Fields::Named(_) => quote! {
                    #vis struct #view_name #view_impl_generics #view_where #decl
                },
                _ => quote! {
                    #vis struct #view_name #view_impl_generics #decl #view_where;
                },
            };
            (
                definition,
                quote!(match from { #pattern => #built }),
                copy_bounds(&data.fields, &field_view),
            )
        }
        Data::Enum(data) => {
            let mut variants = Vec::new();
            let mut arms = Vec::new();
            let mut bounds = Vec::new();
            for variant in &data.variants {
                let v = &variant.ident;
                let (decl, pattern, built) = shape(
                    &variant.fields,
                    &parse_quote!(#name::#v),
                    &parse_quote!(#view_name::#v),
                    &field_view,
                    &field_build,
                )?;
                variants.push(quote!(#v #decl));
                arms.push(quote!(#pattern => #built));
                bounds.extend(copy_bounds(&variant.fields, &field_view));
            }
            let star = data.variants.is_empty().then(|| quote!(*));
            (
                quote! {
                    #vis enum #view_name #view_impl_generics #view_where { #(#variants,)* }
                },
                quote!(match #star from { #(#arms,)* }),
                bounds,
            )
        }
        Data::Union(_) => unreachable!("expand refused a union"),
    };

    let doc = match kind {
        Kind::Ref => format!(
            "A read view of a stored `{name}`, as `sample_ref` returns: tokens at the reader's brand, the rest borrowed."
        ),
        Kind::Mut => format!(
            "A write view of a stored `{name}`: tokens read and written at the writer's brand, the rest borrowed mutably."
        ),
    };
    // A read view is `Copy` when every field's view is, as `&A` is.
    let copy = matches!(kind, Kind::Ref).then(|| {
        let mut copy_generics = view_generics.clone();
        copy_generics
            .make_where_clause()
            .predicates
            .extend(copy_bounds);
        let (copy_impl, _, copy_where) = copy_generics.split_for_impl();
        quote! {
            impl #copy_impl ::core::clone::Clone for #view_name #view_ty_generics #copy_where {
                fn clone(&self) -> Self {
                    *self
                }
            }
            impl #copy_impl ::core::marker::Copy for #view_name #view_ty_generics #copy_where {}
        }
    });

    Ok(quote! {
        #[doc = #doc]
        #definition

        #copy

        impl #impl_impl_generics #trait_path for #name #ty_generics #impl_where {
            type #assoc<#a> = #view_name #view_ty_generics;

            fn #method<#a>(from: #reference Self::Of<'static>, at: ::bough::Loan<#a>) -> Self::#assoc<#a> {
                #body
            }
        }
    })
}

/// `F::Ref<'a>: Copy` for each field of a struct or variant.
fn copy_bounds(
    fields: &Fields,
    field_view: &dyn Fn(&syn::Type, bool) -> Tokens,
) -> Vec<WherePredicate> {
    fields
        .iter()
        .map(|f| {
            let view = field_view(&f.ty, skipped(f).unwrap_or(false));
            parse_quote!(#view: ::core::marker::Copy)
        })
        .collect()
}

/// One struct or variant: its view's fields as declared, the pattern that
/// binds the stored copy's fields, and the view built from them.
fn shape(
    fields: &Fields,
    path: &syn::Path,
    view_path: &syn::Path,
    field_view: &dyn Fn(&syn::Type, bool) -> Tokens,
    field_build: &dyn Fn(&syn::Type, bool, &syn::Ident) -> Tokens,
) -> Result<(Tokens, Tokens, Tokens)> {
    let mut decls = Vec::new();
    let mut binds = Vec::new();
    let mut builds = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        let skip = skipped(field)?;
        let bind = format_ident!("__f{i}");
        let (vis, ty) = (&field.vis, &field.ty);
        let view = field_view(ty, skip);
        decls.push(match &field.ident {
            Some(ident) => quote!(#vis #ident: #view),
            None => quote!(#vis #view),
        });
        builds.push(field_build(ty, skip, &bind));
        binds.push(bind);
    }
    Ok(match fields {
        Fields::Named(named) => {
            let names: Vec<_> = named.named.iter().map(|f| &f.ident).collect();
            (
                quote!({ #(#decls),* }),
                quote!(#path { #(#names: #binds),* }),
                quote!(#view_path { #(#names: #builds),* }),
            )
        }
        Fields::Unnamed(_) => (
            quote!((#(#decls),*)),
            quote!(#path(#(#binds),*)),
            quote!(#view_path(#(#builds),*)),
        ),
        Fields::Unit => (quote!(), quote!(#path), quote!(#view_path)),
    })
}
