//! The examples of Calca's reference (calca.io/reference), and how Notey
//! keeps prose and code safe.

use notey_math::doc::{process, BufState, Mode};

/// Run `doc` and return the text after each `=>`, in order.
fn run(doc: &str, mode: Mode) -> Vec<String> {
    let mut state = BufState::default();
    let out = process(doc, mode, None, &mut state);
    let mut chars: Vec<char> = doc.chars().collect();
    for (s, e, t) in out.edits.iter().rev() {
        chars.splice(*s..*e, t.chars());
    }
    let text: String = chars.into_iter().collect();
    text.split("=>").skip(1).map(|r| statement_end(r.split('\n').next().unwrap()).trim().to_string()).collect()
}

/// Up to a `;` outside brackets.
fn statement_end(s: &str) -> &str {
    let mut depth = 0;
    for (k, c) in s.char_indices() {
        match c {
            '[' | '(' => depth += 1,
            ']' | ')' => depth -= 1,
            ';' if depth == 0 => return &s[..k],
            _ => {}
        }
    }
    s
}

fn check(doc: &str, expected: &[&str]) {
    let got = run(doc, Mode::Calca);
    assert_eq!(got, expected, "\n--- document ---\n{doc}");
}

#[test]
fn therefore() {
    check("y * (1 + 10%) =>\n3 * (1 + 10%) =>", &["1.1y", "3.3"]);
}

#[test]
fn solving() {
    check(
        "y = a*x^2 + b*x + c\n\nx =>\na =>",
        &["[-0.5b/a + 0.5*sqrt(-4a*c + 4a*y + b^2)/a, -0.5b/a - 0.5*sqrt(-4a*c + 4a*y + b^2)/a]", "-b/x - c/x^2 + y/x^2"],
    );
    check("10z + 100x = x^3 + x^2\nx =>", &["100x - x^2 - x^3 == -10z"]);
    check("y(x) = m*x + b\ny(x) = 45\n\nx =>", &["-b/m + 45/m"]);
}

#[test]
fn definitions() {
    check("name = 42\nlong name = -100\n\n5name =>\nname + long name =>", &["210", "-58"]);
}

#[test]
fn functions() {
    check("f(x, y) = x^2 + 2y^2\nf(3, 5) =>\nf(y=5, x=3) =>", &["59", "59"]);
    check(
        "blend = start +\n    t * (end - start)\n\nblend(t = 0) =>\nblend(t = 0.8) =>\nblend(t = 1) =>",
        &["start", "0.8end + 0.2start", "end"],
    );
    check("blend2(t) = start +\n    t * (end - start)\n\nblend2(0.8) =>", &["0.8end + 0.2start"]);
}

#[test]
fn local_definitions() {
    check(
        "rotate(point, angle) =\n    let x = point[0] in\n    let y = point[1] in\n    let ca = cos(angle) in\n    let sa = sin(angle) in\n    [x * ca - y * sa,\n     y * ca + x * sa]\n\nrotate([10, 0], pi/3) =>",
        &["[5, 8.6603]"],
    );
}

#[test]
fn blocks_and_statements() {
    check(
        "long bit of math = this thing + that thing + one other thing\n\nlong bit of math( this thing = 100, that thing = 1) =>",
        &["one other thing + 101"],
    );
    check("err(exp, res) = abs((exp - res)/exp); err(10,12) => 0.2; err(10,11) => 0.1", &["0.2", "0.1"]);
    check("map(cos, [0, pi/4, pi/3])\n  =>", &["[1, 0.7071, 0.5]"]);
}

#[test]
fn headings_scope_definitions() {
    check("##### Level 5\n\na = 10\n\n###### Level 6\n\na =>\n\n##### Another level 5\n\na =>", &["10", "a"]);
}

#[test]
fn cultures() {
    check(
        "a = 3,141.59\n\n#@fr-FR\n\na =>\na*a =>\nb = 12 000,62\n\n#@en-US\n\na =>\nb =>",
        &["3 141,59", "9 869 587,7281", "3,141.59", "12,000.62"],
    );
}

#[test]
fn matrices() {
    check("[1, 2, 3] * 10 =>\n[1, 2, 3] / 10 =>", &["[10, 20, 30]", "[0.1, 0.2, 0.3]"]);
    check("[1, 2, 3] * [1; 2; 3] =>\n[1; 2; 3] * [1, 2, 3] =>", &["[14]", "[1, 2, 3; 2, 4, 6; 3, 6, 9]"]);
    check("[1, 6; 3, 8] + [5, 2; 7, 4] =>\n[1, 6; 3, 8] < [5, 2; 7, 4] =>", &["[6, 8; 10, 12]", "[true, false; true, false]"]);
    check("[foo, bar] * 2 =>", &["[2foo, 2bar]"]);
    check("a = [0, 1; 2, 3]\n\na[0,0] =>\na[1,1] =>\na[1,0] =>\na[0] =>\na[1] =>\na[2] =>", &["0", "3", "2", "0", "2", "1"]);
    check("[1, 2; 3, 4]^T =>", &["[1, 3; 2, 4]"]);
    check("a = [1, 2; 3, 4]\n\na^-1 =>\n|a| =>", &["[-2, 1; 1.5, -0.5]", "-2"]);
    check("a = [1, 2; 3, 4]\na*x = [60; 70]\n\nx =>", &["[-50; 55]"]);
    check(
        "s = [sx, 0, 0; 0, sy, 0; 0, 0, 1]\nt = [1, 0, tx; 0, 1, ty; 0, 0, 1]\n\ns * t * v = [vx; vy; 1]\n\nv =>",
        &["[vx/sx - tx; vy/sy - ty; 1]"],
    );
}

#[test]
fn ranges() {
    check("0:3 =>", &["0:3"]);
    check("a = [1, 2, 3; 4, 5, 6; 7, 8, 9]\na[0:1, 1:2] =>", &["[2, 3; 5, 6]"]);
    check("f(x) = 10x\nmap(f, 0:3) =>", &["[0, 10, 20, 30]"]);
}

#[test]
fn map_and_reduce() {
    check("map (10*x, [0, 1, 500]) =>\nmap (10*y + x, x = [1, 2], y = [3, 4]) =>", &["[0, 10, 5000]", "[31, 42]"]);
    check(
        "reduce (f, [1, 2, 3]) =>\nreduce (acc + x, [1, 2, 3]) =>\nreduce (acc + x + b, [1, 2, 3]) =>\nreduce (f, [1, 2, 3], 1000) =>",
        &["f(f(1, 2), 3)", "6", "2b + 6", "f(f(f(1000, 1), 2), 3)"],
    );
    check(
        "c = [10, 20, 30]\n\nsum (x, c) =>\nreduce (x + y, c) =>\nprod (x, c) =>\nreduce (x * y, c) =>",
        &["60", "60", "6000", "6000"],
    );
}

#[test]
fn logic() {
    check(
        "true && true =>\ntrue && false =>\n1 && 1 =>\n1 && 0 =>\ntrue || false =>\nfalse || false =>\n1 || 0 =>\n0 || 0 =>",
        &["true", "false", "1", "0", "true", "false", "1", "0"],
    );
    check("!true =>\n!(a && (b > 0)) =>", &["false", "!a || (b <= 0)"]);
    check(
        "a(v) = if v < 0 then -v else v\n\na(-100) =>\na(100) =>\na(x + y) =>",
        &["100", "100", "if x + y < 0 then -x - y else x + y"],
    );
}

#[test]
fn computer_numbers() {
    check("0xCC =>\n0xFFFF_FFFF =>\n0b101 =>\n0b1111_1111 =>", &["0xCC", "4294967295", "0b101", "0b11111111"]);
    check("color = round(f*0xFF)\n\ncolor(1/3) =>", &["0x55"]);
    check(
        "0b0101 | 0b1010 =>\n0b0101 & 0b1010 =>\n0b0100 | 1 =>\n0b0100 || 1 =>\n0b0100 & 1 =>\n0b0100 && 1 =>",
        &["0b1111", "0", "0b101", "0b100", "0", "1"],
    );
}

#[test]
fn complex_numbers() {
    check("1 + 2i =>\ni*i =>\n(1+2i)*i =>\ni^43 =>", &["2i + 1", "-1", "i - 2", "-i"]);
    check("sqrt(-10000) =>\nsqrt(i) =>", &["100i", "0.7071i + 0.7071"]);
    check("re(z) = 1/2*(z + conj(z))\nim(z) = 1/2i*(z - conj(z))\n\nre(3 + 5i) =>\nim(3 + 5i) =>", &["3", "5"]);
    check("conj(3i) =>\nconj(2 + 3i) =>\nconj(a) =>", &["-3i", "-3i + 2", "a"]);
}

#[test]
fn derivatives() {
    check(
        "x(t) = 1/2*a*t^2 + v0*t + x0\n\nder(x, t) =>\nder(x, a) =>\nder(x, t, 2) =>",
        &["a*t + v0", "0.5t^2", "a"],
    );
    check("b = 3 + |a - 2|\nder(b) =>", &["if a < 2 then -1 else 1"]);
    check("jacobian(2x/y) =>\njacobian(2x/y, 3x) =>", &["[2/y, -2x/y^2]", "[2/y, -2x/y^2; 3, 0]"]);
    check("jacobian([2x+y, y/x]) =>\njacobian([2x+y, y/x], x, z) =>", &["[2, 1; -y/x^2, 1/x]", "[2, 0; -y/x^2, 0]"]);
    check(
        "f(x) = sin(x) + cos(x)\n\ntaylor(f, x = 0, 1) =>\ntaylor(f, x = 0, 2) =>\ntaylor(f, x = 0, 3) =>",
        &["x + 1", "x - 0.5x^2 + 1", "x - 0.5x^2 - 0.1667x^3 + 1"],
    );
}

#[test]
fn standard_library() {
    check("|-4| =>\n|-x| =>\nabs(-4) =>\nabs(-x) =>\n|[a, b; c, d]| =>", &["4", "|x|", "4", "|x|", "a*d - b*c"]);
    check(
        "acos(0.5) * 180/pi =>\nasin(0.5) * 180/pi =>\natan(0.5) * 180/pi =>\natan2(1, 2) * 180/pi =>",
        &["60", "30", "26.5651", "26.5651"],
    );
    check("cos(0) =>\ncos(pi/3) =>\ncos(pi/4) =>\nsin(pi/3) =>", &["1", "0.5", "0.7071", "0.866"]);
    check("cosh(pi/3) =>\nsinh(pi/4) =>", &["1.6003", "0.8687"]);
    check("ceil(2.1) =>\nceil(-2.1) =>\nfloor(2.1) =>\nfloor(-2.1) =>", &["3", "-2", "2", "-3"]);
    check("dot([a, b], [c, d]) =>\ndot([1, 0], [0.7, 0.3]) =>", &["a*c + b*d", "0.7"]);
    check("inv(2) =>\ninv(a) =>\ninv([1, 2; 3, 4]) =>", &["0.5", "1/a", "[-2, 1; 1.5, -0.5]"]);
    check(
        "log(100, 10) =>\nlog(x, 10) =>\nlog(100, e) =>\nlog(x, e) =>\nlog2(1000) =>\nlog10(1000) =>\nln(1000) =>",
        &["2", "log10(x)", "4.6052", "ln(x)", "9.9658", "3", "6.9078"],
    );
    check("log(x*y, b) =>\nlog(x/y, b) =>", &["log(x, b) + log(y, b)", "log(x, b) - log(y, b)"]);
    check("prod(x, 1:3) =>\nprod(x + a, 1:3) =>\nprod(x + a, a=1:3) =>", &["6", "(a + 1)*(a + 2)*(a + 3)", "(x + 1)*(x + 2)*(x + 3)"]);
    check("prod(x, [1, 2, 3]) =>\nprod(x^2, [1, 2, 3]) =>", &["6", "36"]);
    check("round(2.1) =>\nround(-2.1) =>\nround(1.5) =>\nround(2.5) =>\nround(2.50001) =>", &["2", "-2", "2", "2", "3"]);
    check("sqrt(9) =>\nsqrt(a) =>\na^0.5 =>", &["3", "sqrt(a)", "sqrt(a)"]);
    check("sum(x*x, x=1:5) =>\nsq(v) = v^2\nsum(sq, x=1:5) =>\nsum(sq(x), x=1:5) =>", &["55", "5v^2", "55"]);
    check("truncate(2.1) =>\ntruncate(-2.1) =>", &["2", "-2"]);
}

#[test]
fn currency_without_rates_waits() {
    let mut state = BufState::default();
    let out = process("100 USD in EUR =>", Mode::Prose, None, &mut state);
    assert!(out.edits.is_empty());
    assert!(out.needs_rates && out.currency);
}

#[test]
fn currency_with_rates() {
    let rates = notey_math::currency::Rates::parse(r#"{"base":"EUR","rates":{"USD":1.25,"GBP":0.8}}"#);
    let mut state = BufState::default();
    let doc = "price = $20\nprice * 3 =>\n100 USD in EUR =>\n10 EUR + 10 GBP =>\n€5 in GBP =>\ntotal = $40 + 16 EUR\ntotal =>";
    let out = process(doc, Mode::Prose, rates, &mut state);
    let mut chars: Vec<char> = doc.chars().collect();
    for (s, e, t) in out.edits.iter().rev() {
        chars.splice(*s..*e, t.chars());
    }
    let text: String = chars.into_iter().collect();
    assert_eq!(text, "price = $20\nprice * 3 => 60.00 USD\n100 USD in EUR => 80.00 EUR\n10 EUR + 10 GBP => 22.50 EUR\n€5 in GBP => 4.00 GBP\ntotal = $40 + 16 EUR\ntotal => 60.00 USD");
}

#[test]
fn prose_is_left_alone() {
    let doc = "Monday => call Bob\nPlan => do things\nRevenue - cost => profit\n3 apples => 3 oranges\nStep 1 => Step 2\nx = 4\nx * 2 => 5\n2 + 2 => four\ny = 2z + 1\nz =>";
    assert_eq!(run(doc, Mode::Prose), ["call Bob", "do things", "profit", "3 oranges", "Step 2", "8", "4", "0.5y - 0.5"]);
}

#[test]
fn code_is_left_alone() {
    let doc = "match v {\n    Some(x) => 0,\n    None => 1,\n}\nlet f = |x| x => 2;\nconst g = (a) => a * 2;\nitems.map(x => x + 1)\n2 + 2 =>\nSome(1) =>\nwidth = 40\nwidth * 3 => 7";
    assert_eq!(run(doc, Mode::Code), ["0,", "1,", "2", "a * 2", "x + 1)", "4", "", "7"]);
}

#[test]
fn code_updates_its_own_results() {
    let mut state = BufState::default();
    let apply = |doc: &str, state: &mut BufState| {
        let out = process(doc, Mode::Code, None, state);
        let mut chars: Vec<char> = doc.chars().collect();
        for (s, e, t) in out.edits.iter().rev() {
            chars.splice(*s..*e, t.chars());
        }
        chars.into_iter().collect::<String>()
    };
    let a = apply("x = 2\nx * 3 =>", &mut state);
    assert_eq!(a, "x = 2\nx * 3 => 6");
    let b = apply("x = 5\nx * 3 => 6", &mut state);
    assert_eq!(b, "x = 5\nx * 3 => 15");
    // a fresh tab does not own it
    let c = apply("x = 7\nx * 3 => 15", &mut BufState::default());
    assert_eq!(c, "x = 7\nx * 3 => 15");
}

#[test]
fn markdown_code_blocks_use_code_rules() {
    let doc = "# Notes\n\nTotal: 2 * 21 =>\n\n```js\nconst f = x => x * 2\n```\n2 * 21 =>";
    assert_eq!(run(doc, Mode::Prose), ["", "x * 2", "42"]);
}

#[test]
fn numeric_polynomials() {
    check("x^3 - 6x^2 + 11x - 6 = 0\nx =>", &["[1, 2, 3]"]);
}

#[test]
fn isolates_a_single_occurrence() {
    check("y = 2^x\nx =>", &["log2(y)"]);
}
