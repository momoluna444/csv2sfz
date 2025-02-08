# csv2sfz 

<p dir="rtl">
  English / <a href="https://github.com/momoluna444/csv2sfz/blob/master/README.zh_CN.md">中文</a>
</p>

csv2sfz provides a CSV-based workflow for creating SFZ files. It allows you to manage SFZ data within any spreadsheet GUI, process the data efficiently using glob patterns and math expressions, and easily convert it into SFZ format.

![csv2sfz](https://github.com/user-attachments/assets/103d5c4d-9b40-4f17-8e57-24a30063adc4)

## Features

- **Efficient**: No more redundancy! Use glob patterns to match sample paths and math expressions to calculate opcodes.
- **Simple**: Recursively convert any `.csv` file in the input path to a corresponding `.sfz` file with the same name.
- **Flexible**: No input validation, no output assumptions, can be used for tasks beyond SFZ.

## Installation

This repository provides a C dynamic library and a CLI tool for end users.

### End Users

Download the CLI tool from the Release page.

### Developers

To use csv2sfz as a dependency in a Rust project, add the following to your `Cargo.toml`:

```toml
[dependencies]
csv2sfz = { git = "https://github.com/momoluna444/csv2sfz.git", branch = "master" }
```

For other languages, you need to build from source. After installing the Rust toolchain, follow these steps:
```bash
git clone https://github.com/momoluna444/csv2sfz.git
cd csv2sfz
# Build Lib only
cargo build --release
# Or build both Lib and CLI
cargo build --release --all
```

## CLI Usage
```bash
# Linux / macOS
./csv2sfz /path/to/csv-folder

# Windows
.\csv2sfz.exe /path/to/csv-folder
```

## CSV Usage

### Column Titles

When generating the SFZ file, column titles are typically written as opcodes, for example:

|@header|key|group|off_by|HL3|
|--|--|--|--|--|
|\<region\>|60|1|1|nope|

```c
<region> key=60 group=1 off_by=1 HL3=nope
```
Column title names are flexible, with two key constraints:
- They must be unique.
- They cannot start with double underscores `__`.

Since there is no input validation, make sure that the opcodes are recognized by the SFZ player. You can also leverage this flexibility to achieve things like the following (DecentSampler example):

|@header|@sample(path)|loVel|hiVel|@raw|
|--|--|--|--|--|
|\<sample|"./*.wav"|"1"|"127"|/\>|

```c
<sample path="./Snare.wav" loVel="1" hiVel="127" />
<sample path="./Kick.wav" loVel="1" hiVel="127" />
```

You may have noticed that some special "column titles" are not directly mapped to opcodes, so let's go into detail about them.

### Annotations

#### **@header** - *Required*

Defines the column for "merge range." This annotation must be declared and can only be declared once.

A merge range is defined by the interval `[N, M)`, where the Nth and Mth non-empty strings in column annotated with `@header` mark the boundaries of the range. A column may have multiple merge ranges, for example:

|@header|@sample|lokey|key|*Comment*|
|--|--|--|--|--|
|\<region\>|./*.wav|${k-9}|${k}|*MergeRange 1 Start*|
||./Bass_k60.wav|1||..|
||./Bass_k60.wav||127|..|
|\<region\>|./*.wav|123|123|*MergeRange 1 End / MergeRange 2 Start*|
|||||*(MergeRange 2 End)*|

Rows within the same merge range are evaluated sequentially from top to bottom, and each evaluation merges the result of the current row with the previous row.

Merge rules: The sample path matched by the glob in `@sample` is used as a key. If the key already exists, the new value will overwrite the existing one, while empty values will preserve the previous value. If the key is encountered for the first time, the row is added to the result set.

Each merge range is independent, and its result directly contributes to the final SFZ file. Currently, rows within the same merge range are unordered, but the relative order between rows in different merge ranges is preserved and matches the input order.

The final output of the above example would be:

```c
<region> sample=./Bass_k50.wav lokey=41 key=50 Comment=MergeRange 1 Start
<region> sample=./Bass_k60.wav lokey=1 key=127 Comment=..
<region> sample=./Bass_k70.wav lokey=61 key=70 Comment=MergeRange 1 Start
<region> sample=./Bass_k60.wav lokey=123 key=123 Comment=MergeRange 1 End / MergeRange 2 Start
<region> sample=./Bass_k70.wav lokey=123 key=123 Comment=MergeRange 1 End / MergeRange 2 Start
<region> sample=./Bass_k50.wav lokey=123 key=123 Comment=MergeRange 1 End / MergeRange 2 Start
```

#### **@sample(*\<alias\>*)**

Defines the column for sample paths. This annotation is optional and can only be declared once.

Annotation parameters:
- *<alias\>* - An optional alias for `sample`. By default, `@sample` will output `sample=...`, but you can use `@sample(path)` to modify the opcode to `path=...`.

Columns annotated with `@sample` can use Unix-style glob. The program will match files on disk according to the glob and generate a corresponding row for each matched path. The glob syntax supports:
- `?` matches any single character except `/`.
- `*` matches zero or more characters except `/`.
- `**` matches zero or more characters.
- `{a,b}` matches either pattern `a` or `b`. (Both `a` and `b` can be any glob pattern, but nested `{{...}}` is not supported.)
- `[ab]` matches a single character `a` or `b`
  - Use `[0-9]` to match digits 0 through 1.
  - Use `[!a-zA-Z]` to match any character except lowercase letters `a-z` and uppercase letters `A-Z`.
  - You can escape special characters with square brackets, e.g., `[*]` matches `*`, `[/]` matches `/`.

Additionally, path supports two special syntaxes.
- `// ./Samples/*.wav`: This continues matching files on disk and generates rows for the matched paths, but these rows will not include the `sample` opcode in the SFZ output.
- `"./Samples/*.wav"`: This encloses the output path in quotes.

These special syntaxes can be used with `@header`'s merge range, but note that within the same merge range, you must use one of these syntaxes consistently across all paths. For example:

|@header|@sample|key|
|--|--|--|
|\<region\>|./Bass_k{50,60,70}.wav|1|
||./Bass_k[1-6]0.wav|2|
|\<region\>|// ./Bass_k{50,60}.wav|1|
||// ./Bass_k[1-6]0.wav|2|

```c
<region> sample=./Bass_k50 key=2
<region> sample=./Bass_k60 key=2
<region> sample=./Bass_k70 key=1
<region> key=2
<region> key=2
<region> key=1
```

#### **@raw**

Columns annotated with `@raw` will directly output the content of the cell. This annotation is optional and can be declared multiple times.

|@header|@sample|@raw|@raw|
|--|--|--|--|
|\<region\>|./Bass_k60_ampv127.wav|key=${k}|amp_velcurve_${ampv}=1|

```c
<region> sample=./Bass_k60_ampv127 key=60 amp_velcurve_127=1
```

### Cells

Non-column title cells accept any string as input and support math expressions. Expressions are defined using `${...}`. In addition to basic operators `+`, `-`, `*`, `/`, and `^`, the following builtin functions are supported:
- `sin(x)`, `cos(x)`, `tan(x)`, `asin(x)`, `acos(x)`, `atan(x)`: Trigonometric functions.
- `sqrt(x)`: Square root of `x`.
- `log(x,a)`: Logarithm of `x` with base `a`.
- `abs(x)`: Absolute value of `x`.
- `ceil(x)`: Round `x` up to the nearest integer.
- `floor(x)`: Round `x` down to the nearest integer.
- `round(x,n=0)`: Round `x` to `n` decimal places, default is to return an integer.
- `max(a,b)`: Maximum of `a` and `b`.
- `min(a,b)`: Minimum of `a` and `b`.
- `sat(x)`: Saturates `x` within the range `[0,1]`.
- `vsat(x)`: Saturates `x` within the range `[0,127]`.
- `nl(x,k=-2)`: A nonlinear function for scaling a linear input in the interval `[0,1]`. When `k` is negative, smaller values of `k` cause the output to cluster more densely toward the `1` end of the range. Conversely, when `k` is positive, larger values of `k` result in outputs that are more densely packed near the `0` end. The default value for `k` is `-2`, and the function is defined by the following formula:

$$ f(x,k)=\frac{2^{k \cdot x} - 1}{2^{k} - 1} $$

Expressions can also use parameters declared in the `@sample` file names. The declaration format is `name` `value`, with no separator between the name and value. Parameter names can only include letters, and values can be integers or floats. Multiple parameters are separated by `_`.

For example, a valid file name `Drum_k60_vol1.5_v1_l3.wav` includes parameters `k=60`, `vol=1.5`, `v=1`, and `l=3`. You can use these parameters in expressions, such as in a `@raw` annotated column: `amp_velcurve_${vsat(floor(nl(v/l)*127))}=1`.

## FAQ

### Row Order
For the output SFZ, rows within the same merge range are unordered, but the relative order between different merge ranges is maintained, consistent with the input.

### Column Order
For the output SFZ, columns are ordered, consistent with the input.

### File Name Parameters
Parameter names can only contain letters, and parameter values must be integers or floats. Parameters have no specific order and can be mixed with non-parameter text, as long as they are separated by `_`.

### Math Expressions
Input is limited to integers or floats, and calculations are performed using 64-bit floating point precision. Invalid expressions will be directly output in the SFZ for debugging purposes.

### Column Titles
Except for special annotations, each column title must be unique. Duplicate column titles will lead to unexpected behavior. Internally, the program uses double underscores `__` to handle column titles like `@raw` that should not be output, so avoid using double underscores at the start of column title names. Additionally, empty column titles and their columns will not be output.
