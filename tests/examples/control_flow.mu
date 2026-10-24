/* Spec §8: conditionals, loops, switch, when, scripts and leap. */
word Light { red, yellow, green }

score f(l: Light) -> i32 {
    as #score: 0
    switch (l) {
        case Light.red    { #score += 1 }  then
        case Light.yellow { #score += 10 } break
        default {}
    }
    when (l) {
        is red                          { #score += 2 }
        is yellow where (#score > 20)   { #score -= 1 }
        is _                            {}
    }
    as #i: 0
    do { #i += 1 } until (#i == 3)
    while (#i > 0) { #i -= 1 }
    if (#score > 10) { #score := 10 } else if (#score < 0) { #score := 0 } else { #score += 1 }
    => #score
}

find f(a[4, i32]) -> i32 {
    as #i: 0
    while (#i < a[]) {
        if (a[#i] == 0) { leap not_found }
        #i += 1
    }
    => 1
not_found:
    => 0
}

tick f() -> effect {
    as #steps: 0
    script bump { #steps += 1 }
    do bump
    do bump                         /* #steps == 2 */
    => done
}
