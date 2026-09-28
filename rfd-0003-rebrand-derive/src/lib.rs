//! `#[derive(Rebrand)]`, for `rfd-0003-brand-erasure`: can a derive write
//! the `Rebrand` impls that probe wrote by hand, including a copy-free
//! `view` for a type with no brand?
//!
//! The trait is the one in `fixtures/rfd-0003-brand-erasure/api.rs`, reached
//! as `::bough::Rebrand`, as a real derive reaches its own crate:
//!
//! ```text
//! pub trait Rebrand: Sized {
//!     type Of<'x>: 'x;
//!     fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x>;
//!     fn restore<'x>(from: &Self::Of<'x>, at: Witness<'x>) -> Self;
//!     fn view<'a>(from: &'a Self::Of<'static>) -> Option<&'a Self> { None }
//! }
//! ```
//!
//! What it writes, for `struct Pair<'g, T> { cell: Cell<'g, u32>, value: T }`:
//!
//! ```text
//! impl<'g, T: ::bough::Rebrand> ::bough::Rebrand for Pair<'g, T> {
//!     type Of<'x> = Pair<'x, <T as ::bough::Rebrand>::Of<'x>>;
//!     fn rebrand<'x>(&self, to: Witness<'x>) -> Self::Of<'x> {
//!         match self {
//!             Pair { cell: f0, value: f1 } => Pair {
//!                 cell: <Cell<'g, u32> as Rebrand>::rebrand(f0, to),
//!                 value: <T as Rebrand>::rebrand(f1, to),
//!             },
//!         }
//!     }
//!     // `restore` is the same, from `Of<'x>` back to `Self`.
//! }
//! ```
//!
//! The choices:
//!
//! - **The brand is the type's one lifetime parameter, whatever its name.**
//!   A stored value is kept at `Of<'static>`, which must be `'static`, so a
//!   `Rebrand` type can't hold any other borrow; a second lifetime is an
//!   error here, with a message, rather than an impl that fails later.
//! - **`Of<'x>` is the type with its brand renamed to `'x` and each type
//!   parameter `T` replaced by `T::Of<'x>`.** Each field is rebranded through
//!   its own impl, so this compiles exactly when every field's `Of` is its
//!   type under the same renaming, which the api's impls for tokens and std
//!   containers all are. A field whose type isn't `Rebrand` is an error at
//!   that field.
//! - **`view` is `Some(from)` when the type has no lifetime and no type
//!   parameter**: then `Of<'x>` is the type itself. A generic type loses it
//!   even when instantiated brand-free (`Tagged<u32>`), because the impl
//!   can't prove `T::Of<'static>` is `T`.
//! - **`#[rebrand(skip)]` marks a leaf field**: cloned across brands, and
//!   its type is the same in `Self` and in `Of`, so a skipped field that
//!   holds the brand or a type parameter doesn't compile. It is the field
//!   level `Leaf`, for another crate's type in a struct.
//!
//! With the `views` feature it also writes the borrowed views of
//! `rfd-0003-brand-erasure -- borrow`; `views.rs` says how.
use proc_macro::TokenStream;
use proc_macro2::{Span, TokenStream as Tokens};
use quote::{format_ident, quote, quote_spanned};
use syn::spanned::Spanned;
use syn::{
    Data, DeriveInput, Error, Fields, GenericParam, Lifetime, Path, Result, parse_macro_input,
    parse_quote,
};

#[proc_macro_derive(Rebrand, attributes(rebrand))]
pub fn derive_rebrand(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

fn expand(input: &DeriveInput) -> Result<Tokens> {
    let name = &input.ident;
    let brand = brand(input)?;
    // The method lifetime, distinct from the brand so it can't shadow it.
    let x = match &brand {
        Some(b) if b.ident == "x" => Lifetime::new("'y", Span::call_site()),
        _ => Lifetime::new("'x", Span::call_site()),
    };

    // `impl<'g, T: Rebrand>`, and `Of<'x> = Name<'x, T::Of<'x>>`.
    let mut generics = input.generics.clone();
    let mut has_type_param = false;
    // `Of<'x>` is the same type at `T::Of<'x>`, so it must meet the type's
    // own bounds there: `T: Clone` needs `for<'x> T::Of<'x>: Clone`.
    let mut of_bounds: Vec<syn::WherePredicate> = Vec::new();
    for param in &mut generics.params {
        if let GenericParam::Type(t) = param {
            has_type_param = true;
            let (ident, bounds) = (&t.ident, &t.bounds);
            if !bounds.is_empty() {
                of_bounds.push(parse_quote!(
                    for<#x> <#ident as ::bough::Rebrand>::Of<#x>: #bounds
                ));
            }
            t.bounds.push(parse_quote!(::bough::Rebrand));
        }
    }
    let type_params: Vec<_> = input.generics.type_params().map(|t| &t.ident).collect();
    for predicate in input
        .generics
        .where_clause
        .iter()
        .flat_map(|w| &w.predicates)
    {
        if let syn::WherePredicate::Type(p) = predicate
            && let syn::Type::Path(path) = &p.bounded_ty
            && let Some(ident) = path.path.get_ident()
            && type_params.contains(&ident)
        {
            let bounds = &p.bounds;
            of_bounds.push(parse_quote!(
                for<#x> <#ident as ::bough::Rebrand>::Of<#x>: #bounds
            ));
        }
    }
    generics.make_where_clause().predicates.extend(of_bounds);
    let (impl_generics, _, where_clause) = generics.split_for_impl();
    let (_, ty_generics, _) = input.generics.split_for_impl();
    let of_args = input.generics.params.iter().map(|param| match param {
        GenericParam::Lifetime(_) => quote!(#x),
        GenericParam::Type(t) => {
            let t = &t.ident;
            quote!(<#t as ::bough::Rebrand>::Of<#x>)
        }
        GenericParam::Const(c) => {
            let c = &c.ident;
            quote!(#c)
        }
    });
    let of = if input.generics.params.is_empty() {
        quote!(#name)
    } else {
        quote!(#name<#(#of_args),*>)
    };

    let (rebrand, restore) = match &input.data {
        Data::Struct(data) => {
            let path: Path = parse_quote!(#name);
            let (pattern, rebuilt, restored) = arm(&path, &data.fields)?;
            (
                quote!(match self { #pattern => #rebuilt }),
                quote!(match from { #pattern => #restored }),
            )
        }
        Data::Enum(data) => {
            let mut rebrands = Vec::new();
            let mut restores = Vec::new();
            for variant in &data.variants {
                let v = &variant.ident;
                let path: Path = parse_quote!(#name::#v);
                let (pattern, rebuilt, restored) = arm(&path, &variant.fields)?;
                rebrands.push(quote!(#pattern => #rebuilt));
                restores.push(quote!(#pattern => #restored));
            }
            // An empty enum matches through the reference.
            let star = data.variants.is_empty().then(|| quote!(*));
            (
                quote!(match #star self { #(#rebrands,)* }),
                quote!(match #star from { #(#restores,)* }),
            )
        }
        Data::Union(u) => {
            return Err(Error::new(
                u.union_token.span(),
                "Rebrand can't be derived for a union: its fields can't be rebuilt one by one",
            ));
        }
    };

    // Copy-free reads, for a type whose `Of` is itself.
    let view = (brand.is_none() && !has_type_param).then(|| {
        quote! {
            fn view<'a>(from: &'a Self::Of<'static>) -> ::core::option::Option<&'a Self> {
                ::core::option::Option::Some(from)
            }
        }
    });

    let rebrand_impl = quote! {
        impl #impl_generics ::bough::Rebrand for #name #ty_generics #where_clause {
            type Of<#x> = #of;

            fn rebrand<#x>(&self, to: ::bough::Witness<#x>) -> Self::Of<#x> {
                #rebrand
            }

            fn restore<#x>(from: &Self::Of<#x>, at: ::bough::Witness<#x>) -> Self {
                #restore
            }

            #view
        }
    };
    #[cfg(feature = "views")]
    let rebrand_impl = {
        let views = views::expand(input, &generics, brand.as_ref(), has_type_param)?;
        quote!(#rebrand_impl #views)
    };
    Ok(rebrand_impl)
}

#[cfg(feature = "views")]
mod views;

/// The brand: the type's one lifetime parameter, or none.
fn brand(input: &DeriveInput) -> Result<Option<Lifetime>> {
    let mut lifetimes = input.generics.lifetimes();
    let brand = lifetimes.next().map(|l| l.lifetime.clone());
    if let Some(second) = lifetimes.next() {
        return Err(Error::new(
            second.span(),
            format!(
                "Rebrand: a type may have one lifetime, its brand; `{}` is a second. \
                 A value is stored at brand 'static, so it can't hold a borrow",
                second.lifetime
            ),
        ));
    }
    Ok(brand)
}

/// One struct or variant: the pattern binding its fields, the value rebuilt
/// at brand `'x` from them, and the value restored from them.
fn arm(path: &Path, fields: &Fields) -> Result<(Tokens, Tokens, Tokens)> {
    let mut binds = Vec::new();
    let mut rebrands = Vec::new();
    let mut restores = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        let bind = format_ident!("__f{i}");
        let ty = &field.ty;
        // Spanned at the field's type, so a missing impl is reported there.
        let (rebrand, restore) = if skipped(field)? {
            (
                quote_spanned!(ty.span()=> <#ty as ::core::clone::Clone>::clone(#bind)),
                quote_spanned!(ty.span()=> <#ty as ::core::clone::Clone>::clone(#bind)),
            )
        } else {
            (
                quote_spanned!(ty.span()=> <#ty as ::bough::Rebrand>::rebrand(#bind, to)),
                quote_spanned!(ty.span()=> <#ty as ::bough::Rebrand>::restore(#bind, at)),
            )
        };
        rebrands.push(rebrand);
        restores.push(restore);
        binds.push(bind);
    }
    Ok(match fields {
        Fields::Named(named) => {
            let names: Vec<_> = named.named.iter().map(|f| &f.ident).collect();
            (
                quote!(#path { #(#names: #binds),* }),
                quote!(#path { #(#names: #rebrands),* }),
                quote!(#path { #(#names: #restores),* }),
            )
        }
        Fields::Unnamed(_) => (
            quote!(#path(#(#binds),*)),
            quote!(#path(#(#rebrands),*)),
            quote!(#path(#(#restores),*)),
        ),
        Fields::Unit => (quote!(#path), quote!(#path), quote!(#path)),
    })
}

/// Whether a field is marked `#[rebrand(skip)]`.
fn skipped(field: &syn::Field) -> Result<bool> {
    let mut skip = false;
    for attr in &field.attrs {
        if !attr.path().is_ident("rebrand") {
            continue;
        }
        attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") {
                skip = true;
                Ok(())
            } else {
                Err(meta.error("unknown rebrand option; the one option is `skip`"))
            }
        })?;
    }
    Ok(skip)
}
