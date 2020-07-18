use super::SafeName;
use crate::ast::{ArraySize, ArrayType, BasicType, Node};
use crate::indexes::{AstType, GenericIndex, TypeIndex};
use crate::Result;
use std::collections::BTreeMap;

enum TypeResolve {
    None,
    ChaseTypedefs,
}

impl TypeResolve {
    fn skip_resolve(&self) -> bool {
        match self {
            Self::None => true,
            Self::ChaseTypedefs => false,
        }
    }
}

pub fn print_impl_from<'a, W: std::fmt::Write>(
    w: &mut W,
    item: &Node,
    generic_index: &GenericIndex<'a>,
    constant_index: &BTreeMap<&str, String>,
    type_index: &TypeIndex<'a>,
) -> Result<()> {
    match item {
        Node::EOF => {}
        Node::Root(v) => {
            for field in v.iter() {
                print_impl_from(w, field, generic_index, constant_index, type_index)?;
            }
        }
        Node::Struct(v) => print_try_from(w, v.name.as_str(), generic_index, |w| {
            writeln!(w, "Ok({} {{", v.name)?;
            for f in v.fields.iter() {
                write!(w, "{}: ", SafeName(&f.field_name))?;
                if f.is_optional {
                    // This field is an optional field
                    //
                    // Outputs:
                    // 		match v.try_i32()? {
                    // 			0 => None,
                    // 			1 => Some(Box::new(TYPE::try_from(v)?)),
                    // 			d => return Err(Error::UnknownVariant(d as i32)),
                    // 		}
                    writeln!(w, "{{ match v.try_u32()? {{")?;
                    writeln!(w, "0 => None,")?;
                    writeln!(
                        w,
                        "1 => Some(Box::new({}::try_from(v)?)),",
                        f.field_value.unwrap_array()
                    )?;
                    writeln!(w, "d => return Err(Error::UnknownOptionVariant(d)),")?;
                    writeln!(w, "}}}},")?;
                } else {
                    // Struct fields that are typedefs should not be chased.
                    if let BasicType::Ident(i) = f.field_value.unwrap_array() {
                        if let Some(AstType::Typedef(td)) = type_index.get(i) {
                            print_decode_basic_type(w, &td.alias, type_index, TypeResolve::None)?;
                            writeln!(w, "?,")?;
                            continue;
                        }
                    }

                    print_decode_array(
                        w,
                        &f.field_value,
                        type_index,
                        constant_index,
                        generic_index,
                    )?;
                    writeln!(w, ",")?;
                }
            }
            writeln!(w, "}})")?;
            Ok(())
        })?,

        Node::Union(v) => print_try_from(w, v.name.as_str(), generic_index, |w| {
            write!(w, "let {} = ", SafeName(&v.switch.var_name))?;
            print_decode_basic_type(
                w,
                &v.switch.var_type,
                type_index,
                TypeResolve::ChaseTypedefs,
            )?;
            writeln!(w, "?;")?;

            writeln!(w, "Ok(match {} {{", SafeName(&v.switch.var_name))?;
            for c in v.cases.iter() {
                // A single case statement may have many case values tied to it
                // if fallthrough values are used:
                //
                // 	case 1:
                // 	case 2:
                // 		// statement
                //
                for c_value in c.case_values.iter() {
                    // The case value may be a declared constant or enum value.
                    //
                    // Lookup the value in the `constant_index`.
                    let matcher = constant_index.get(c_value.as_str()).unwrap_or(c_value);

                    write!(w, "{} => Self::{}(", SafeName(matcher), c.field_name)?;
                    print_decode_array(
                        w,
                        &c.field_value,
                        type_index,
                        constant_index,
                        generic_index,
                    )?;
                    writeln!(w, "),")?;
                }
            }

            // There may also be several "void" cases
            for c in v.void_cases.iter() {
                let name = match c.as_str() {
                    "default" => "_",
                    v => &v,
                };
                writeln!(w, "{} => Self::Void,", name)?;
            }

            // Write a default case if present, else a catch all case that
            // returns an error.
            if let Some(ref d) = v.default {
                write!(w, "_ => Self::{}(", SafeName(&d.field_name))?;
                print_decode_array(w, &d.field_value, type_index, constant_index, generic_index)?;
                writeln!(w, "),")?;
            } else {
                writeln!(w, "d => return Err(Error::UnknownVariant(d as i32)),")?;
            }

            writeln!(w, "}})")?;

            Ok(())
        })?,

        Node::Enum(v) => print_try_from(w, v.name.as_str(), generic_index, |w| {
            writeln!(w, "Ok(match v.try_i32()? {{")?;
            for variant in v.variants.iter() {
                writeln!(w, "{} => Self::{},", variant.value, variant.name)?;
            }
            writeln!(w, "d => return Err(Error::UnknownVariant(d as i32)),\n}})")?;
            Ok(())
        })?,

        Node::Typedef(v) => print_try_from(w, v.alias.as_str(), generic_index, |w| {
            write!(w, "Ok(Self(")?;
            print_decode_basic_type(w, &v.alias, type_index, TypeResolve::ChaseTypedefs)?;
            writeln!(w, "?))")?;
            Ok(())
        })?,

        Node::Constant(_) => return Ok(()),

        Node::Ident(_)
        | Node::Type(_)
        | Node::Option(_)
        | Node::UnionDefault(_)
        | Node::UnionCase(_)
        | Node::UnionDataField(_)
        | Node::UnionVoid
        | Node::StructDataField(_)
        | Node::Array(_)
        | Node::EnumVariant(_)
        | Node::ArrayVariable(_)
        | Node::ArrayFixed(_) => unreachable!(),
    };

    Ok(())
}

/// Prints the "impl TryFrom" block around the output of func.
///
/// `func` should write the body of the `try_from` implementation to `w`, using
/// `v` as the Bytes source.
fn print_try_from<'a, W: std::fmt::Write, F: FnOnce(&mut W) -> Result<()>>(
    mut w: W,
    name: &str,
    generic_index: &GenericIndex,
    func: F,
) -> Result<()> {
    if generic_index.contains(name) {
        write!(w, "impl TryFrom<Bytes> for {}<Bytes>", name)?;
    } else {
        write!(w, r#"impl TryFrom<Bytes> for {}"#, name)?;
    }
    writeln!(
        w,
        r#" {{
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {{"#
    )?;

    func(&mut w)?;

    writeln!(w, "}}\n}}")?;

    Ok(())
}

fn print_decode_array<'a, W>(
    w: &mut W,
    t: &ArrayType<BasicType<'a>>,
    type_index: &TypeIndex<'a>,
    constant_index: &BTreeMap<&str, String>,
    generic_index: &GenericIndex<'a>,
) -> Result<()>
where
    W: std::fmt::Write,
{
    // Print a fixed-size array.
    let print_fixed = |w: &mut W, t: &BasicType, size: u32| -> Result<()> {
        match t {
            BasicType::Opaque => write!(w, "v.try_bytes({})?", size)?,
            BasicType::String => unreachable!("unexpected fixed length string"),
            _ => {
                writeln!(w, "[")?;
                for _i in 0..size {
                    print_decode_basic_type(w, t, type_index, TypeResolve::ChaseTypedefs)?;
                    writeln!(w, "?,")?;
                }
                write!(w, "]")?;
            }
        }
        Ok(())
    };

    // Print a length-prefixed variable sized array, or a string with a maximum value.
    let print_variable = |w: &mut W, t: &BasicType, size: Option<u32>| -> Result<()> {
        let mut type_str = t.to_string();

        if generic_index.contains(type_str.as_str()) {
            type_str = format!("{}<Bytes>", type_str);
        };

        let size = size
            .map(|s| format!("Some({})", s))
            .unwrap_or("None".to_string());

        match t {
            BasicType::Opaque => write!(w, "v.try_variable_bytes({})?", size)?,
            BasicType::String => write!(w, "v.try_string({})?", size)?,
            _ => write!(w, "v.try_variable_array::<{}>({})?", type_str, size)?,
        };

        Ok(())
    };

    match t {
        ArrayType::None(t) => {
            print_decode_basic_type(w, t, type_index, TypeResolve::ChaseTypedefs)?;
            write!(w, "?")?;
        }
        ArrayType::FixedSize(t, ArraySize::Known(size)) => print_fixed(w, t, *size)?,
        ArrayType::FixedSize(t, ArraySize::Constant(size)) => {
            // Try and resolve the constant value
            let size = constant_index
                .get(size.as_str())
                .ok_or(format!("unknown constant {}", size))?;

            print_fixed(w, t, size.parse()?)?
        }
        ArrayType::VariableSize(t, Some(ArraySize::Known(size))) => {
            print_variable(w, t, Some(*size))?
        }
        ArrayType::VariableSize(t, Some(ArraySize::Constant(size))) => {
            // Try and resolve the constant value
            let size = constant_index
                .get(size.as_str())
                .ok_or("unknown constant")?;

            print_variable(w, t, Some(size.parse()?))?;
        }
        ArrayType::VariableSize(t, None) => {
            print_variable(w, t, None)?;
        }
    };

    Ok(())
}

/// Generates the template required to decode `t` from a variable called `v`
/// that implements the reader trait.
///
/// If `t` is a typedef alias, the typedef chain is resolved to the underlying
/// type.
fn print_decode_basic_type<'a, W>(
    w: &mut W,
    t: &BasicType<'a>,
    type_index: &TypeIndex<'a>,
    resolve_typedefs: TypeResolve,
) -> Result<()>
where
    W: std::fmt::Write,
{
    match t {
        BasicType::U32 => write!(w, "v.try_u32()")?,
        BasicType::U64 => write!(w, "v.try_u64()")?,
        BasicType::I32 => write!(w, "v.try_i32()")?,
        BasicType::I64 => write!(w, "v.try_i64()")?,
        BasicType::F32 => write!(w, "v.try_f32()")?,
        BasicType::F64 => write!(w, "v.try_f64()")?,
        BasicType::Bool => write!(w, "v.try_bool()")?,
        BasicType::String => write!(w, "v.try_string(None)")?,
        BasicType::Opaque => write!(w, "v.try_variable_bytes(None)")?,

        // If typedefs should not be resolved to their targets (for struct
        // fields) just print a try_from() impl for the ident name.
        BasicType::Ident(c) if resolve_typedefs.skip_resolve() => write!(w, "{}::try_from(v)", c)?,

        // An ident may refer to a typedef, or a compound type.
        BasicType::Ident(c) => match type_index.get(c) {
            Some(AstType::Basic(ref b)) => {
                return print_decode_basic_type(w, b, type_index, resolve_typedefs)
            }
            Some(AstType::Struct(s)) => write!(w, "{}::try_from(v)", s.name())?,
            Some(AstType::Union(u)) => write!(w, "{}::try_from(v)", u.name())?,
            Some(AstType::Enum(e)) => write!(w, "{}::try_from(v)", e.name)?,

            // If this typedef should not be chased, print the alias try_from() impl.
            Some(AstType::Typedef(t)) if resolve_typedefs.skip_resolve() => {
                write!(w, "{}::try_from(v)", t.alias)?
            }

            // Otherwise print the target's try_from, but only go one level down
            // the typedef chain.
            Some(AstType::Typedef(t)) => {
                return print_decode_basic_type(w, &t.target, type_index, TypeResolve::None)
            }

            None => return Err(format!("unresolvable type {}", c.as_ref()).into()),
        },
    };
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::indexes::*;
    use crate::{walk, Rule, XDRParser};
    use pest::Parser;

    macro_rules! test_convert {
        ($name: ident, $input: expr, $want: expr) => {
            #[test]
            fn $name() {
                let mut ast = XDRParser::parse(Rule::item, $input).unwrap();
                let ast = walk(ast.next().unwrap()).unwrap();
                let constant_index = build_constant_index(&ast);
                let generic_index = build_generic_index(&ast);
                let type_index = TypeIndex::new(&ast);

                let mut got = String::new();
                print_impl_from(&mut got, &ast, &generic_index, &constant_index, &type_index)
                    .unwrap();

                assert_eq!(got, $want);
            }
        };
    }

    test_convert!(
        test_struct_basic_types,
        r#"
			struct small {
				unsigned int a;
				unsigned hyper b;
				int c;
				hyper d;
				float e;
				double f;
				string g;
				bool h;
				opaque i;
			};
		"#,
        r#"impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_u32()?,
b: v.try_u64()?,
c: v.try_i32()?,
d: v.try_i64()?,
e: v.try_f32()?,
f: v.try_f64()?,
g: v.try_string(None)?,
h: v.try_bool()?,
i: v.try_variable_bytes(None)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_opaque_fields,
        r#"
            const SIZE = 3;
			struct small {
				opaque a;
				opaque b<>;
				opaque c<42>;
				opaque c_c<SIZE>;
				opaque d[2];
				opaque d_c[SIZE];
			};
		"#,
        r#"impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_bytes(None)?,
b: v.try_variable_bytes(None)?,
c: v.try_variable_bytes(Some(42))?,
c_c: v.try_variable_bytes(Some(3))?,
d: v.try_bytes(2)?,
d_c: v.try_bytes(3)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_string_fields,
        r#"
			struct small {
				string a;
				string b<>;
				string c<42>;
			};
		"#,
        r#"impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_string(None)?,
b: v.try_string(None)?,
c: v.try_string(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_struct_option,
        r#"
			struct other {
				u32 b;
			};
			struct small {
				other *a;
			};
		"#,
        r#"impl TryFrom<Bytes> for other {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: { match v.try_u32()? {
0 => None,
1 => Some(Box::new(other::try_from(v)?)),
d => return Err(Error::UnknownOptionVariant(d)),
}},
})
}
}
"#
    );

    test_convert!(
        test_struct_option_self_referential,
        r#"
			struct small {
				small *a;
			};
		"#,
        r#"impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: { match v.try_u32()? {
0 => None,
1 => Some(Box::new(small::try_from(v)?)),
d => return Err(Error::UnknownOptionVariant(d)),
}},
})
}
}
"#
    );

    test_convert!(
        test_struct_reserved_keyword,
        r#"
			struct small {
				unsigned int type;
			};
		"#,
        r#"impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
type_v: v.try_u32()?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_struct,
        r#"
			struct other {
				u32 b;
			};
			struct small {
				other a;
			};
		"#,
        r#"impl TryFrom<Bytes> for other {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: other::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_struct_generic,
        r#"
			struct other {
				opaque b;
			};
			struct small {
				other a;
			};
		"#,
        r#"impl TryFrom<Bytes> for other<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_variable_bytes(None)?,
})
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: other::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_typedef_to_struct,
        r#"
			typedef other alias;
			struct other {
				u32 b;
			};
			struct small {
				alias a;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(other::try_from(v)?))
}
}
impl TryFrom<Bytes> for other {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: alias::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_typedef_to_struct_generic,
        r#"
			typedef other alias;
			struct other {
				opaque b;
			};
			struct small {
				alias a;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(other::try_from(v)?))
}
}
impl TryFrom<Bytes> for other<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_variable_bytes(None)?,
})
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: alias::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_typedef_to_basic_type,
        r#"
			typedef u32 alias;
			struct small {
				alias a;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_u32()?))
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: alias::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_typedef_to_basic_type_generic,
        r#"
			typedef opaque alias;
			struct small {
				alias a;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_variable_bytes(None)?))
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: alias::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_typedef_to_union,
        r#"
			typedef my_union alias;
			union my_union switch (unsigned int status) {
			case 1:
				u32       resok4;
			};
			struct small {
				alias a;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(my_union::try_from(v)?))
}
}
impl TryFrom<Bytes> for my_union {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: alias::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_typedef_to_union_generic,
        r#"
			typedef my_union alias;
			union my_union switch (unsigned int status) {
			case 1:
				opaque       resok4;
			};
			struct small {
				alias a;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(my_union::try_from(v)?))
}
}
impl TryFrom<Bytes> for my_union<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: alias::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_union,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			case 2:
				u64       name;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::name(v.try_u64()?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_constant_case_value,
        r#"
			const MODE4_SUID = 0x800;
			union CB_GETATTR4res switch (unsigned int status) {
			case MODE4_SUID:
				u32       resok4;
			case 2:
				u64       name;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
0x800 => Self::resok4(v.try_u32()?),
2 => Self::name(v.try_u64()?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_enum_case_value,
        r#"
			enum Status{
				MODE4_SUID = 1,
				MODE4_OTHER = 2
			};
			union CB_GETATTR4res switch (unsigned int status) {
			case MODE4_SUID:
				u32       resok4;
			case 2:
				u64       name;
			};
		"#,
        r#"impl TryFrom<Bytes> for Status {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(match v.try_i32()? {
1 => Self::MODE4_SUID,
2 => Self::MODE4_OTHER,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
Status::MODE4_SUID => Self::resok4(v.try_u32()?),
2 => Self::name(v.try_u64()?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_reserved_keyword_variant_ignored,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			case 2:
				u64       type;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::type(v.try_u64()?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_with_fallthrough,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			case 2:
			case 3:
				u64       name;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::name(v.try_u64()?),
3 => Self::name(v.try_u64()?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_default_case,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			default:
				u64       name;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
_ => Self::name(v.try_u64()?),
})
}
}
"#
    );

    test_convert!(
        test_union_default_case_with_fallthrough,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			case 2:
			default:
				u64       name;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
_ => Self::name(v.try_u64()?),
})
}
}
"#
    );

    // This case isn't optimal - there's two wildcard branches but the first one
    // is the void, so this works fine and is simpler to generate.
    test_convert!(
        test_union_default_void,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			default:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
_ => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    // This case isn't optimal - there's two wildcard branches but the first one
    // is the void, so this works fine and is simpler to generate.
    test_convert!(
        test_union_default_void_with_fallthrough,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			case 2:
			default:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::Void,
_ => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_void_case,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_void_case_with_fallthrough,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			case 2:
			case 3:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::Void,
3 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_struct,
        r#"
			struct simple {
				u32 a;
			};
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				simple       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for simple {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(simple {
a: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(simple::try_from(v)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_union,
        r#"
			union my_union switch (unsigned int status) {
			case 1:
				u32       var;
			};
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				my_union       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for my_union {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::var(v.try_u32()?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(my_union::try_from(v)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_union_generic,
        r#"
			union my_union switch (unsigned int status) {
			case 1:
				opaque       var;
			};
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				my_union       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for my_union<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::var(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_GETATTR4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(my_union::try_from(v)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_typedef_to_union,
        r#"
			typedef my_union alias;
			union my_union switch (unsigned int status) {
			case 1:
				u32       var;
			};
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				alias       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(my_union::try_from(v)?))
}
}
impl TryFrom<Bytes> for my_union {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::var(v.try_u32()?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(my_union::try_from(v)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_typedef_to_union_generic,
        r#"
			typedef my_union alias;
			union my_union switch (unsigned int status) {
			case 1:
				opaque       var;
			};
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				alias       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(my_union::try_from(v)?))
}
}
impl TryFrom<Bytes> for my_union<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::var(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_GETATTR4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(my_union::try_from(v)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_typedef_to_struct,
        r#"
			typedef small alias;
			struct small {
				u32 a;
			};
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				alias       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(small::try_from(v)?))
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(small::try_from(v)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_typedef_to_struct_generic,
        r#"
			typedef small alias;
			struct small {
				opaque a;
			};
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				alias       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(small::try_from(v)?))
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_bytes(None)?,
})
}
}
impl TryFrom<Bytes> for CB_GETATTR4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(small::try_from(v)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_typedef_to_basic_type,
        r#"
			typedef u32 alias;
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				alias       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_u32()?))
}
}
impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_typedef_to_basic_type_generic,
        r#"
			typedef opaque alias;
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				alias       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_variable_bytes(None)?))
}
}
impl TryFrom<Bytes> for CB_GETATTR4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_variable_bytes(None)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_basic_type,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				u32       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_nested_basic_type_generic,
        r#"
			union CB_GETATTR4res switch (unsigned int status) {
			case 1:
				opaque       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_variable_bytes(None)?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_switch_typedef,
        r#"
			typedef u32 alias;
			union CB_GETATTR4res switch (alias status) {
			case 1:
				u32       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_u32()?))
}
}
impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let status = v.try_u32()?;
Ok(match status {
1 => Self::resok4(v.try_u32()?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_reserved_fieldname,
        r#"
			union CB_GETATTR4res switch (u32 type) {
			case 1:
				u32       resok4;
			case 2:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for CB_GETATTR4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let type_v = v.try_u32()?;
Ok(match type_v {
1 => Self::resok4(v.try_u32()?),
2 => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_union_switch_enum,
        r#"
			enum time_how4 {
				SET_TO_SERVER_TIME4 = 0,
				SET_TO_CLIENT_TIME4 = 1
			};

			union settime4 switch (time_how4 set_it) {
			case SET_TO_CLIENT_TIME4:
				u32       time;
			default:
				void;
			};
		"#,
        r#"impl TryFrom<Bytes> for time_how4 {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(match v.try_i32()? {
0 => Self::SET_TO_SERVER_TIME4,
1 => Self::SET_TO_CLIENT_TIME4,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for settime4 {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let set_it = time_how4::try_from(v)?;
Ok(match set_it {
time_how4::SET_TO_CLIENT_TIME4 => Self::time(v.try_u32()?),
_ => Self::Void,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
"#
    );

    test_convert!(
        test_fixed_array_known,
        r#"
            struct small {
                uint32_t a[3];
            };
        "#,
        r#"impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: [
v.try_u32()?,
v.try_u32()?,
v.try_u32()?,
],
})
}
}
"#
    );

    test_convert!(
        test_fixed_array_known_struct,
        r#"
            struct other {
                u32 b;
            };
            struct small {
                other a[2];
            };
        "#,
        r#"impl TryFrom<Bytes> for other {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: [
other::try_from(v)?,
other::try_from(v)?,
],
})
}
}
"#
    );

    test_convert!(
        test_fixed_array_const,
        r#"
            const SIZE = 2;
            struct small {
                uint32_t a[SIZE];
            };
        "#,
        r#"impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: [
v.try_u32()?,
v.try_u32()?,
],
})
}
}
"#
    );

    test_convert!(
        test_fixed_array_const_struct,
        r#"
            const SIZE = 2;
            struct other {
                u32 b;
            };
            struct small {
                other a[SIZE];
            };
        "#,
        r#"impl TryFrom<Bytes> for other {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: [
other::try_from(v)?,
other::try_from(v)?,
],
})
}
}
"#
    );

    test_convert!(
        test_variable_array_no_max,
        r#"
            struct small {
                uint32_t a<>;
            };
        "#,
        r#"impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_array::<u32>(None)?,
})
}
}
"#
    );

    test_convert!(
        test_variable_array_no_max_struct,
        r#"
            struct other {
                u32 b;
            };
            struct small {
                other a<>;
            };
        "#,
        r#"impl TryFrom<Bytes> for other {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_array::<other>(None)?,
})
}
}
"#
    );

    test_convert!(
        test_variable_array_max_known,
        r#"
            struct small {
                uint32_t a<42>;
            };
        "#,
        r#"impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_array::<u32>(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_variable_array_max_known_struct,
        r#"
            struct other {
                u32 b;
            };
            struct small {
                other a<42>;
            };
        "#,
        r#"impl TryFrom<Bytes> for other {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_array::<other>(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_variable_array_max_const,
        r#"
            const SIZE = 42;
            struct small {
                uint32_t a<SIZE>;
            };
        "#,
        r#"impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_array::<u32>(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_variable_array_max_const_struct,
        r#"
            const SIZE = 42;
            struct other {
                u32 b;
            };
            struct small {
                other a<SIZE>;
            };
        "#,
        r#"impl TryFrom<Bytes> for other {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_u32()?,
})
}
}
impl TryFrom<Bytes> for small {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_array::<other>(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_fixed_array_known_struct_generic,
        r#"
                struct other {
                    opaque b;
                };
                struct small {
                    other a[2];
                };
            "#,
        r#"impl TryFrom<Bytes> for other<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_variable_bytes(None)?,
})
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: [
other::try_from(v)?,
other::try_from(v)?,
],
})
}
}
"#
    );

    test_convert!(
        test_fixed_array_const_struct_generic,
        r#"
                const SIZE = 2;
                struct other {
                    opaque b;
                };
                struct small {
                    other a[SIZE];
                };
            "#,
        r#"impl TryFrom<Bytes> for other<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_variable_bytes(None)?,
})
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: [
other::try_from(v)?,
other::try_from(v)?,
],
})
}
}
"#
    );

    test_convert!(
        test_variable_array_max_known_struct_generic,
        r#"
            struct other {
                opaque b;
            };
            struct small {
                other a<42>;
            };
        "#,
        r#"impl TryFrom<Bytes> for other<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_variable_bytes(None)?,
})
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_array::<other<Bytes>>(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_variable_array_max_const_struct_generic,
        r#"
            const SIZE = 42;
            struct other {
                opaque b;
            };
            struct small {
                other a<SIZE>;
            };
        "#,
        r#"impl TryFrom<Bytes> for other<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(other {
b: v.try_variable_bytes(None)?,
})
}
}
impl TryFrom<Bytes> for small<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(small {
a: v.try_variable_array::<other<Bytes>>(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_enum,
        r#"
            enum a_status {
                ZERO          = 0,
                ONE           = 1
            };

            struct DELEGPURGE4res {
                a_status        status;
            };
        "#,
        r#"impl TryFrom<Bytes> for a_status {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(match v.try_i32()? {
0 => Self::ZERO,
1 => Self::ONE,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for DELEGPURGE4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(DELEGPURGE4res {
status: a_status::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_typedef_enum,
        r#"
            typedef a_status alias;
            enum a_status {
                ZERO          = 0,
                ONE           = 1
            };

            struct DELEGPURGE4res {
                alias        status;
            };
        "#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(a_status::try_from(v)?))
}
}
impl TryFrom<Bytes> for a_status {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(match v.try_i32()? {
0 => Self::ZERO,
1 => Self::ONE,
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for DELEGPURGE4res {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(DELEGPURGE4res {
status: alias::try_from(v)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_to_typedef_array_fixed_union_generic,
        r#"
            typedef opaque alias;
            union u_type_name switch (unsigned int s) {
                case 1:    alias some_var;
            };
            struct CB_COMPOUND4res {
                u_type_name   resarray[2];
            };
        "#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_variable_bytes(None)?))
}
}
impl TryFrom<Bytes> for u_type_name<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let s = v.try_u32()?;
Ok(match s {
1 => Self::some_var(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_COMPOUND4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(CB_COMPOUND4res {
resarray: [
u_type_name::try_from(v)?,
u_type_name::try_from(v)?,
],
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_to_typedef_array_variable_max_union_generic,
        r#"
            typedef opaque alias;
            union u_type_name switch (unsigned int s) {
                case 1:    alias some_var;
            };
            struct CB_COMPOUND4res {
                u_type_name   resarray<42>;
            };
        "#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_variable_bytes(None)?))
}
}
impl TryFrom<Bytes> for u_type_name<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let s = v.try_u32()?;
Ok(match s {
1 => Self::some_var(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_COMPOUND4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(CB_COMPOUND4res {
resarray: v.try_variable_array::<u_type_name<Bytes>>(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_to_typedef_array_variable_no_max_union_generic,
        r#"
            typedef opaque alias;
            union u_type_name switch (unsigned int s) {
                case 1:    alias some_var;
            };
            struct CB_COMPOUND4res {
                u_type_name   resarray<>;
            };
        "#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_variable_bytes(None)?))
}
}
impl TryFrom<Bytes> for u_type_name<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let s = v.try_u32()?;
Ok(match s {
1 => Self::some_var(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_COMPOUND4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(CB_COMPOUND4res {
resarray: v.try_variable_array::<u_type_name<Bytes>>(None)?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_to_array_fixed_union_generic,
        r#"
            union u_type_name switch (unsigned int s) {
                case 1:    opaque some_var;
            };
            struct CB_COMPOUND4res {
                u_type_name   resarray[2];
            };
        "#,
        r#"impl TryFrom<Bytes> for u_type_name<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let s = v.try_u32()?;
Ok(match s {
1 => Self::some_var(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_COMPOUND4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(CB_COMPOUND4res {
resarray: [
u_type_name::try_from(v)?,
u_type_name::try_from(v)?,
],
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_to_array_variable_max_union_generic,
        r#"
            union u_type_name switch (unsigned int s) {
                case 1:    opaque some_var;
            };
            struct CB_COMPOUND4res {
                u_type_name   resarray<42>;
            };
        "#,
        r#"impl TryFrom<Bytes> for u_type_name<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let s = v.try_u32()?;
Ok(match s {
1 => Self::some_var(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_COMPOUND4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(CB_COMPOUND4res {
resarray: v.try_variable_array::<u_type_name<Bytes>>(Some(42))?,
})
}
}
"#
    );

    test_convert!(
        test_struct_nested_to_array_variable_no_max_union_generic,
        r#"
            union u_type_name switch (unsigned int s) {
                case 1:    opaque some_var;
            };
            struct CB_COMPOUND4res {
                u_type_name   resarray<>;
            };
        "#,
        r#"impl TryFrom<Bytes> for u_type_name<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
let s = v.try_u32()?;
Ok(match s {
1 => Self::some_var(v.try_variable_bytes(None)?),
d => return Err(Error::UnknownVariant(d as i32)),
})
}
}
impl TryFrom<Bytes> for CB_COMPOUND4res<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(CB_COMPOUND4res {
resarray: v.try_variable_array::<u_type_name<Bytes>>(None)?,
})
}
}
"#
    );

    test_convert!(
        test_typedef_basic_type,
        r#"
            typedef uint32_t alias;
        "#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_u32()?))
}
}
"#
    );

    test_convert!(
        test_typedef_opaque,
        r#"
            typedef opaque alias;
        "#,
        r#"impl TryFrom<Bytes> for alias<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_variable_bytes(None)?))
}
}
"#
    );

    test_convert!(
        test_typedef_complex_type,
        r#"
            typedef target alias;
            struct target {
                uint32_t var_name;
            };
        "#,
        r#"impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(target::try_from(v)?))
}
}
impl TryFrom<Bytes> for target {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(target {
var_name: v.try_u32()?,
})
}
}
"#
    );

    test_convert!(
        test_typedef_typedef_basic_type,
        r#"
            typedef alias alias2;
            typedef uint32_t alias;
        "#,
        r#"impl TryFrom<Bytes> for alias2 {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(alias::try_from(v)?))
}
}
impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_u32()?))
}
}
"#
    );

    test_convert!(
        test_typedef_typedef_complex_type,
        r#"
            typedef alias alias2;
            typedef target alias;
            struct target {
                uint32_t var_name;
            };
        "#,
        r#"impl TryFrom<Bytes> for alias2 {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(alias::try_from(v)?))
}
}
impl TryFrom<Bytes> for alias {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(target::try_from(v)?))
}
}
impl TryFrom<Bytes> for target {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(target {
var_name: v.try_u32()?,
})
}
}
"#
    );

    test_convert!(
        test_struct_typedef_structs,
        r#"
            typedef uint32_t        acemask4;
            typedef utf8string      utf8str_mixed;
            typedef opaque  utf8string<>;
            struct nfsace4 {
                acemask4                access_mask;
                utf8str_mixed           who;
            };
        "#,
        r#"impl TryFrom<Bytes> for acemask4 {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_u32()?))
}
}
impl TryFrom<Bytes> for utf8str_mixed<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(utf8string::try_from(v)?))
}
}
impl TryFrom<Bytes> for utf8string<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(Self(v.try_variable_bytes(None)?))
}
}
impl TryFrom<Bytes> for nfsace4<Bytes> {
type Error = Error;

fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
Ok(nfsace4 {
access_mask: acemask4::try_from(v)?,
who: utf8str_mixed::try_from(v)?,
})
}
}
"#
    );

    //     test_convert!(
    //         test_struct_typedef_array_generic,
    //         r#"
    //             typedef opaque alias;
    //             struct small {
    //                 alias   field<>;
    //             };
    //         "#,
    //         r#"impl TryFrom<Bytes> for small<Bytes> {
    // type Error = Error;

    // fn try_from(mut v: Bytes) -> Result<Self, Self::Error> {
    // Ok(small {
    // resarray: v.try_variable_bytes(None)?,
    // })
    // }
    // }
    // "#
    //     );
}

// TODO: typedefs as types
