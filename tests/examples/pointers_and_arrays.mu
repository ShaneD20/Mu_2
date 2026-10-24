/* Spec §4.2 and §4.3: arrays with lengths in their types, thin and fat pointers. */
sum f(a[10, i32]) -> i32 {
    as #grid[4, i32]: [5, 6, 7, 8]
    #grid[1] := 42
    #grid := [1, 2, 3, 4]
    as n: a[]
    as value: 5
    as *p[i32]: &value
    as *q[len, i32]: &a
    as *r[10, i32]: &a
    as count: *q[]
    as first: *q[0]
    as doubled: value * *p
    as same: &value == p
    => doubled
}
