/* Spec §4.4–4.5, §9–§13: tuples, aliases, functions, structs, words, unions,
   interfaces and prerun. */
share struct Person {
    height: -1.0,                   /* immutable field */
    #age: 1,                        /* mutable field */
    greet f()[height] -> f64 { => height }
    bump# f()[#age] -> effect { #age += 1; => done }
}

word Level(u8) { low: 1, mid: 5, high: 9 }
word Err(u16, start: 256) { First, Second }
share word Code(u8) { ok: 0, bad: 1 }

union Shape(Circle, Square) : Drawable {
    type area: f() -> f64           /* a contract */
}

struct Shape.Circle { 
    radius: 1.0,  
    area f()[radius] -> f64 { => radius * radius * 3.1415 } 
}
struct Shape.Square { 
    side: 1.0,    
    area f()[side]   -> f64 { => side * side } 
}

interface Drawable {
    type area: f() -> f64
    type scale: f(f64) -> f64
}

type unary: f(i64) -> i64
type pure_op: f(i64)[!] -> i64
type bar: i16

as #total: 0
tally# f(x: i32)[#total] -> effect { #total += x; => done }

total_area f(s: Shape) -> f64 { => s.area() }

outer f(n: i64) -> i64 {
    helper f(x: i64)[n] -> i64 { => x + n }
    as double: (unary) \(x) { => 2 * x }
    => helper(double(n))
}

sum_to f(n: i64, acc: i64) -> i64 {
    => so(n == 0) acc else sum_to(n - 1, acc + n)
}

people f() -> effect {
    as linda: Person()
    as #sam: Person(5.5, #age: 12)
    #sam.bump#()
    bump(ref #sam)
    => done
}

prerun square f(x: i32) -> i32 { => x * x }
as table_size: square(12)

as pair: (i32, f32) (1000, 1.25)
as first: pair[0]
as level: Level(5)
as code: (u8) Code.ok
as gain: (q16.16) 1
as letter: 'x'
