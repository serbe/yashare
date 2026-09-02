/// Generates a field enum for Yandex.Disk API response fields.
///
/// This macro creates an enum that represents fields in Yandex.Disk's
/// `?fields` query parameter syntax. Each variant encodes a field path that
/// can be rendered into the comma-separated field list expected by the API.
///
/// # Syntax
/// ```text
/// field_enum! {
///     pub enum Name {
///         leaf {
///             VARIANT => "field.path",
///             ...
///         }
///         nested {
///             VARIANT(OtherEnum) => "parent.field",
///             ...
///         }
///     }
/// }
/// ```
///
/// # Generated code
/// The macro generates:
/// - The enum type with `leaf` and `nested` variants.
/// - An implementation of `FieldPath` that renders the field path to a string.
/// - An `all()` method that returns a `Vec` of all variants (including nested ones).
///
/// # Example
/// ```rust
/// field_enum! {
///     pub enum ResourceField {
///         leaf {
///             Name => "name",
///             Path => "path",
///         }
///         nested {
///             Embedded(EmbeddedField) => "_embedded",
///         }
///     }
/// }
/// ```
///
/// This generates `ResourceField::Name` rendering as `"name"`,
/// and `ResourceField::Embedded(EmbeddedField::Items)` rendering as
/// `"_embedded.items"`.
#[macro_export]
macro_rules! field_enum {
    (
        $vis:vis enum $name:ident {
            leaf { $( $lvariant:ident => $lpath:literal ),* $(,)? }
            nested { $( $nvariant:ident($ntype:ty) => $npath:literal ),* $(,)? }
        }
    ) => {
        #[derive(Debug, Clone)]
        $vis enum $name {
            $( $lvariant, )*
            $( $nvariant($ntype), )*
        }

        impl FieldPath for $name {
            fn write(&self, out: &mut String) {
                match self {
                    $( Self::$lvariant => out.push_str($lpath), )*
                    $(
                        Self::$nvariant(inner) => {
                            out.push_str($npath);
                            out.push('.');
                            inner.write(out);
                        }
                    )*
                }
            }
        }

        impl $name {
            /// Returns a `Vec` containing all possible variants of this field enum.
            ///
            /// This includes both leaf variants and all nested variants from
            /// the nested field types. The order is deterministic and follows
            /// the order of declaration.
            pub fn all() -> Vec<Self> {
                let result: Vec<Self> = vec![$( Self::$lvariant ),*];
                $(
                    let result: Vec<Self> = result
                        .into_iter()
                        .chain(<$ntype>::all().into_iter().map(Self::$nvariant))
                        .collect();
                )*
                result
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use crate::model::{ResourceField, fields::FieldPath};

    field_enum! {
        enum Inner {
            leaf { X => "x" }
            nested {}
        }
    }

    field_enum! {
        enum Sample {
            leaf {
                A => "a",
                B => "b",
            }
            nested {
                C(Inner) => "c",
            }
        }
    }

    #[test]
    fn renders_leaf_path() {
        assert_eq!(Sample::A.to_path(), "a");
    }

    #[test]
    fn renders_nested_path() {
        assert_eq!(Sample::C(Inner::X).to_path(), "c.x");
    }

    #[test]
    fn all_covers_leaf_and_nested_variants() {
        let paths: Vec<String> = Sample::all().iter().map(FieldPath::to_path).collect();
        assert_eq!(paths, vec!["a", "b", "c.x"]);
    }

    #[test]
    fn empty_leaf_or_nested_block_compiles() {
        field_enum! {
            enum OnlyLeaf {
                leaf { Only => "only" }
                nested {}
            }
        }
        assert_eq!(OnlyLeaf::Only.to_path(), "only");
    }

    #[test]
    fn default_matches_all() {
        let default_set: HashSet<_> =
            ResourceField::default().iter().map(|f| f.to_path()).collect();
        let all_set: HashSet<_> = ResourceField::all().iter().map(|f| f.to_path()).collect();
        assert!(default_set.is_subset(&all_set));
    }
}
