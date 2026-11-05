# Math

Live math in any Notey tab, in the style of [Calca](https://calca.io/reference).
Write `=>` after a calculation and the result appears after it, updated as
you type.

```
price = $20
tax = 8%
price * (1 + tax) * 3 => 64.80 USD
```

## Writing math

- **Results**: `expression =>` writes the result after the `=>`. Text after
  a `=>` is the result, so it gets replaced.
- **Definitions**: `name = value`. Names can have several words
  (`long name = 5`). A name can't be defined twice: a second `name = ...`
  is an equation, used for solving.
- **Functions**: `f(x, y) = x^2 + 2y^2`, then `f(3, 5) =>` or
  `f(y=5, x=3) =>`. Every definition is a function of its unknowns:
  `blend = start + t*(end - start)`, then `blend(t = 0.8) =>`.
- **Unknowns stay symbols**: `y * (1 + 10%) => 1.1y`.
- **Solving**: `x =>` with an undefined `x` solves the latest equation that
  uses it: linear and quadratic equations exactly, and polynomials with
  numeric coefficients numerically.
- **Several lines**: indent the lines that continue a calculation. Separate
  statements on one line with `;`. `#` starts a comment.
- **Headings** (`# Title`) scope definitions to their section and its
  subsections.
- **Numbers**: `2x`, `10%`, `1e6`, `0xFF` and `0b1010` (32-bit), `i` for
  complex numbers, `pi`, `e`. `#@fr-FR` switches the decimal comma and
  digit grouping; `#@` switches back.
- **Matrices**: `[1, 2; 3, 4]`, `m[row, col]`, `m[i]`, `m^T`, `m^-1`, `|m|`
  (determinant). `*` multiplies matrices; other operators work per element.
- **Ranges**: `0:3`, also for parts of matrices: `m[0:1, 1:2]`.
- **Logic**: `&&`, `||`, `!`, comparisons, `if c then a else b`.
  `&` and `|` work on bits. `|x|` is the absolute value.
- **Local names**: `let x = value in expression`.

## Functions

`abs`, `sqrt`, `exp`, `ln`, `log(x, base)`, `log2`, `log10`, `sin`, `cos`,
`tan`, `asin`, `acos`, `atan`, `atan2`, `sinh`, `cosh`, `tanh`, `floor`,
`ceil`, `round` (halves to even), `truncate`, `sign`, `mod`, `min`, `max`,
`conj`, `inv`, `det`, `transpose`, `dot`, and:

- `map(f, list)`, `map(10*x, list)`, `map(x + y, x = list, y = list)`
- `reduce(f, list)` and `reduce(acc + x, list, start)`
- `sum(x^2, x = 1:5)` and `prod(x, list)`
- `der(f, x)`, `der(f, x, 2)`: derivatives (piecewise for `|x|`)
- `taylor(f, x = 0, 3)`: Taylor series
- `jacobian(f, g, ...)`, `jacobian([f, g], x, y)`

## Money

Currency codes and symbols are units: `100 USD`, `$20`, `€5`, `£3`, `¥500`.
Convert with `in`: `100 USD in EUR =>`. A sum of different currencies is
shown in the first one written. Rates are the European Central Bank's daily
reference rates (31 currencies), downloaded through
[Frankfurter](https://frankfurter.dev) when a tab uses money, and refreshed
twice a day. **Edit > Math > Update Exchange Rates** fetches them now.
This is the plugin's only network use.

## Where Math writes

Math runs in every tab, but only writes where that is safe:

- **`.calca` files** work exactly like Calca.
- **Plain text and Markdown** get a result when it says something new: a
  value from numbers, definitions or functions. A line like
  `Monday => call Bob` is never touched.
- **Code files** and Markdown code blocks get only plain values (numbers,
  money, matrices), and only after a `=>` with nothing after it, or a
  result Math wrote while the tab was open. `Some(x) => 0` in Rust or
  `x => x * 2` in JavaScript is left alone.

**Edit > Math > Turn Math Off or On in This Tab** pauses it for one tab
(with the menu bar hidden: **Plugins > Math** in the Notey menu).
Results that follow your typing undo with it.

## Not supported

Calca's Google search (`#?`) and units of measure other than money.

## Source

The engine is `math.wasm`, built from `crates/notey-math` in the
[Notey repository](https://github.com/deepfold9118/notey) with
`cargo xtask math`. It runs sandboxed: it can only compute, with no access
to files or the network.
