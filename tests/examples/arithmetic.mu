/* Spec §2.3 and §6: literals, suffixed operators, casts and cast blocks. */
math f(a: i32, b: i32) -> i32 {
    as c: a + b                 /* default: traps on overflow */
    as d: a +% b                /* wrap */
    as e: a +| b                /* saturate */
    as #x: (i64) 5
    #x +%= 1
    as n: (i8) -128
    as m: (i8)% c               /* truncates */
    as k: (i8)| c               /* saturates */
    as total: (i64) {
        a +
        b +
        c
    }
    as big: 1_000_000 * 0xFF_FF
    as ratio: 1.5e-3 * 2E+10
    as mask: 0b1010 & ~d
    as clamped: so(c < 0) 0 else so(c > 9) 9 else c
    as t: (so(a > b) a else b) + 1
    => so(a > b)
        a else
        b
}
