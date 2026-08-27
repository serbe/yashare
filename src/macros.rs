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
    use crate::model::fields::FieldPath;

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
}
