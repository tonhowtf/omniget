//! Content stream operators.

use crate::content::{Instruction, OPERANDS_THRESHOLD, OperatorTrait, Stack};
use crate::object;
use crate::object::Array;
use crate::object::Name;
use crate::object::Number;
use crate::object::Object;
use crate::object::Stream;
use smallvec::{SmallVec, smallvec};

use crate::content::macros::{op_all, op_impl, op0, op1, op2, op3, op4, op6};

include!("ops_generated.rs");

// Need to special-case those because they have variable arguments.

fn parse_named_color<'b, 'a>(
    objects: &'b [Object<'a>],
) -> Option<(SmallVec<[Number; OPERANDS_THRESHOLD]>, Option<&'b Name<'a>>)> {
    let mut nums = smallvec![];
    let mut name = None;

    for o in objects {
        match o {
            Object::Number(n) => nums.push(*n),
            Object::Name(n) => name = Some(n),
            _ => {
                warn!("encountered unknown object {o:?} when parsing scn/SCN");

                return None;
            }
        }
    }

    Some((nums, name))
}

#[derive(Debug, Clone, PartialEq)]
pub struct StrokeColorNamed<'b, 'a>(
    pub SmallVec<[Number; OPERANDS_THRESHOLD]>,
    pub Option<&'b Name<'a>>,
);

op_impl!(
    StrokeColorNamed<'b, 'a>,
    "SCN",
    u8::MAX as usize,
    |stack: &'b Stack<'a>| {
        let (nums, name) = parse_named_color(stack.as_slice())?;
        Some(StrokeColorNamed(nums, name))
    }
);

#[derive(Debug, PartialEq, Clone)]
pub struct NonStrokeColorNamed<'b, 'a>(
    pub SmallVec<[Number; OPERANDS_THRESHOLD]>,
    pub Option<&'b Name<'a>>,
);

op_impl!(
    NonStrokeColorNamed<'b, 'a>,
    "scn",
    u8::MAX as usize,
    |stack: &'b Stack<'a>| {
        let (nums, name) = parse_named_color(stack.as_slice())?;
        Some(NonStrokeColorNamed(nums, name))
    }
);

#[cfg(test)]
mod tests {
    use crate::content::TypedIter;
    use crate::content::ops::{
        BeginMarkedContentWithProperties, ClosePath, EndMarkedContent, FillPathNonZero, LineTo,
        MarkedContentPointWithProperties, MoveTo, NonStrokeColorDeviceRgb, NonStrokeColorNamed,
        SetGraphicsState, StrokeColorNamed, Transform, TypedInstruction,
    };
    use crate::object::Name;
    use crate::object::Number;
    use crate::object::Object;
    use crate::object::{Dict, FromBytes};
    fn n(num: i32) -> Number {
        Number::from_i32(num)
    }

    #[test]
    fn basic_ops_1() {
        let input = b"
1 0 0 -1 0 200 cm
/g0 gs
1 0 0 rg
";

        let mut iter = TypedIter::new(input);

        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::Transform(Transform(a, b, c, d, e, f)))
                if [a, b, c, d, e, f] == [n(1), n(0), n(0), n(-1), n(0), n(200)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::SetGraphicsState(SetGraphicsState(name)))
                if name.as_ref() == b"g0"
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::NonStrokeColorDeviceRgb(NonStrokeColorDeviceRgb(r, g, b)))
                if [r, g, b] == [n(1), n(0), n(0)]
        ));
        assert!(iter.next().is_none());
    }

    #[test]
    fn basic_ops_2() {
        let input = b"
20 20 m
180 20 l
180 180 l
20 180 l
h
f
";

        let mut iter = TypedIter::new(input);

        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::MoveTo(MoveTo(x, y))) if [x, y] == [n(20), n(20)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::LineTo(LineTo(x, y))) if [x, y] == [n(180), n(20)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::LineTo(LineTo(x, y))) if [x, y] == [n(180), n(180)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::LineTo(LineTo(x, y))) if [x, y] == [n(20), n(180)]
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::ClosePath(ClosePath))
        ));
        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::FillPathNonZero(FillPathNonZero))
        ));
        assert!(iter.next().is_none());
    }

    /// PdfCraft patch (28): a twelve-ink DeviceN colour's `scn` keeps all twelve operands, and
    /// the instructions after it are still read (#4260). The eleventh operand ended the stream.
    #[test]
    fn scn_with_twelve_components_keeps_them_all() {
        let input = b"1 2 3 4 5 6 7 8 9 10 11 12 scn 0 0 10 10 re f";
        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::NonStrokeColorNamed(NonStrokeColorNamed(nums, None))) => {
                let expected = (1..=12).map(Number::from_i32).collect::<Vec<_>>();
                assert_eq!(nums.as_slice(), expected.as_slice());
            }
            other => panic!("unexpected instruction: {other:?}"),
        }
        assert!(matches!(iter.next(), Some(TypedInstruction::RectPath(_))));
        assert!(matches!(iter.next(), Some(TypedInstruction::FillPathNonZero(_))));
        assert!(iter.next().is_none());
    }

    /// PdfCraft patch (28): past the operand cap the oldest operands are dropped, in order, and
    /// the operator reads the ones nearest to it.
    #[test]
    fn operands_past_the_cap_keep_the_latest_in_order() {
        let mut input = String::new();
        for i in 0..600 {
            input.push_str(&format!("{i} "));
        }
        input.push_str("scn 0 1 0 rg");
        let mut iter = TypedIter::new(input.as_bytes());

        match iter.next() {
            Some(TypedInstruction::NonStrokeColorNamed(NonStrokeColorNamed(nums, None))) => {
                let expected = (344..600).map(Number::from_i32).collect::<Vec<_>>();
                assert_eq!(nums.as_slice(), expected.as_slice());
            }
            other => panic!("unexpected instruction: {other:?}"),
        }
        assert!(matches!(iter.next(), Some(TypedInstruction::NonStrokeColorDeviceRgb(_))));
        assert!(iter.next().is_none());
    }

    #[test]
    fn scn() {
        let input = b"
0.0 scn
1.0 1.0 1.0 /DeviceRgb SCN
";

        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::NonStrokeColorNamed(NonStrokeColorNamed(nums, None))) => {
                assert_eq!(nums.as_slice(), &[Number::from_f32(0.0)]);
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        match iter.next() {
            Some(TypedInstruction::StrokeColorNamed(StrokeColorNamed(nums, Some(name)))) => {
                assert_eq!(
                    nums.as_slice(),
                    &[
                        Number::from_f32(1.0),
                        Number::from_f32(1.0),
                        Number::from_f32(1.0)
                    ]
                );
                assert_eq!(name.as_ref(), b"DeviceRgb");
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        assert!(iter.next().is_none());
    }

    #[test]
    fn dp() {
        let input = b"/Attribute<</ShowCenterPoint false >> DP";

        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::MarkedContentPointWithProperties(
                MarkedContentPointWithProperties(name, object),
            )) => {
                assert_eq!(name.as_ref(), b"Attribute");
                assert_eq!(
                    object,
                    &Object::Dict(Dict::from_bytes(b"<</ShowCenterPoint false >>").unwrap())
                );
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        assert!(iter.next().is_none());
    }

    #[test]
    fn bdc_with_dict() {
        let input = b"/Span << /MCID 0 /Alt (Alt)>> BDC EMC";

        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::BeginMarkedContentWithProperties(
                BeginMarkedContentWithProperties(name, object),
            )) => {
                assert_eq!(name.as_ref(), b"Span");
                assert_eq!(
                    object,
                    &Object::Dict(Dict::from_bytes(b"<< /MCID 0 /Alt (Alt)>>").unwrap())
                );
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::EndMarkedContent(EndMarkedContent))
        ));
        assert!(iter.next().is_none());
    }

    #[test]
    fn bdc_with_name() {
        let input = b"/Span /Name BDC EMC";

        let mut iter = TypedIter::new(input);

        match iter.next() {
            Some(TypedInstruction::BeginMarkedContentWithProperties(
                BeginMarkedContentWithProperties(name, object),
            )) => {
                assert_eq!(name.as_ref(), b"Span");
                assert_eq!(object, &Object::Name(Name::new_unescaped(b"Name")));
            }
            other => panic!("unexpected instruction: {other:?}"),
        }

        assert!(matches!(
            iter.next(),
            Some(TypedInstruction::EndMarkedContent(EndMarkedContent))
        ));
        assert!(iter.next().is_none());
    }
}
