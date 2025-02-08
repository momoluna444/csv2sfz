# csv2sfz 

<p dir="rtl">
  中文 / <a href="https://github.com/momoluna444/csv2sfz">English</a>
</p>

该工具提供一种基于CSV的SFZ工作流。你可以在任何表格GUI中管理SFZ数据，使用glob模式匹配与数学表达式高效处理数据，并轻松转换为SFZ格式。

![csv2sfz](https://github.com/user-attachments/assets/103d5c4d-9b40-4f17-8e57-24a30063adc4)

## 特性

- **高效**：不再重复！用glob模式匹配编写样本路径，用数学表达式计算opcode。
- **简单**：递归的将输入路径内的任何`.csv`文件转换为同名`.sfz`文件。
- **灵活**：无输入校验，无输出预设，可用于SFZ之外的任务。

## 安装

该仓库提供C动态库和面向普通用户的CLI程序。

### 普通用户

前往Release页面下载CLI二进制文件。

### 开发者

对于Rust，要使csv2sfz成为依赖，可在 `Cargo.toml` 中添加下列代码：

```toml
[dependencies]
csv2sfz = { git = "https://github.com/momoluna444/csv2sfz.git", branch = "master" }
```

对于其他语言，要从源码构建。安装 Rust 工具链后，按照下列说明操作：
```bash
git clone https://github.com/momoluna444/csv2sfz.git
cd csv2sfz
# Build Lib only
cargo build --release
# Or build both Lib and CLI
cargo build --release --all
```


## CLI用法
```bash
# Linux / macOS
./csv2sfz /path/to/csv-folder

# Windows
.\csv2sfz.exe /path/to/csv-folder
```

## CSV用法

### 列标题

在输出时，列标题通常会被当作opcode写入SFZ文件，如：

|@header|key|group|off_by|HL3|
|--|--|--|--|--|
|\<region\>|60|1|1|nope|

```c
<region> key=60 group=1 off_by=1 HL3=nope
```
列标题的名称很自由，只有两点要求：
- 不能重复。
- 不能使用双下划线`__`作为前缀。

由于输入没有验证，你需要确保它们能被播放器识别！同时你也可以利用这种灵活性做这样的事（DecentSampler示例）：

|@header|@sample(path)|loVel|hiVel|@raw|
|--|--|--|--|--|
|\<sample|"./*.wav"|"1"|"127"|/\>|

```c
<sample path="./Snare.wav" loVel="1" hiVel="127" />
<sample path="./Kick.wav" loVel="1" hiVel="127" />
```

你应该发现了，有些特殊的“列标题”并不会被直接映射为opcode，接下来让我们详细介绍它们。

### 注解

#### **@header** - *必要*

定义“合并范围”的所在列。该注解必须被声明且仅能声明一次。

一个合并范围指，在`@header`标注的列中，第N个非空字符串到最近的第M个非空字符串，所组成的闭开区间`[N,M)`。一个列中可拥有复数个合并范围，例如：

|@header|@sample|lokey|key|*Comment*|
|--|--|--|--|--|
|\<region\>|./*.wav|${k-9}|${k}|*MergeRange 1 Start*|
||./Bass_k60.wav|1||..|
||./Bass_k60.wav||127|..|
|\<region\>|./*.wav|123|123|*MergeRange 1 End / MergeRange 2 Start*|
|||||*(MergeRange 2 End)*|

同一个合并范围内的行，会被按照从上到下的顺序依次进行评估，且每一次评估都会将当前行的结果与上一行的结果进行合并。

合并规则为：以`@sample`中glob匹配的采样路径为键，若已存在相同的键，按照“新值盖旧值，空值留旧值”的原则进行合并；若当前键为第一次出现，则直接将该行插入结果集。

每个合并范围都是独立的，它们的结果将直接贡献给最终输出的SFZ文件。目前，对于合并范围的输出结果，同一个合并范围内的行是无序的，但不同合并范围的行之间的相对顺序为有序，且与输入顺序相同。

上例的最终输出为：

```c
<region> sample=./Bass_k50.wav lokey=41 key=50 Comment=MergeRange 1 Start
<region> sample=./Bass_k60.wav lokey=1 key=127 Comment=..
<region> sample=./Bass_k70.wav lokey=61 key=70 Comment=MergeRange 1 Start
<region> sample=./Bass_k60.wav lokey=123 key=123 Comment=MergeRange 1 End / MergeRange 2 Start
<region> sample=./Bass_k70.wav lokey=123 key=123 Comment=MergeRange 1 End / MergeRange 2 Start
<region> sample=./Bass_k50.wav lokey=123 key=123 Comment=MergeRange 1 End / MergeRange 2 Start
```

#### **@sample(*\<alias\>*)**

定义路径的所在列。该注解为可选，仅能声明一次。

注解参数：
- *<alias\>* - 可选的`sample`别名。默认情况下，`@sample`会输出`sample=...`，你可以使用`@sample(path)`来修改输出的opcode名称为`path=...`。

被`@sample`标注的列，可以使用Unix风格的glob模式匹配。程序会根据glob对磁盘上的文件进行匹配，并为每个匹配的路径生成对应的行。glob支持下列语法：
- `?`匹配除`/`外的任意单个字符。
- `*`匹配除`/`外的零个或多个字符。
- `**`匹配零个或多个字符。
- `{a,b}`匹配模式`a`或`b`。（`a`和`b`为任意glob模式，但不支持嵌套`{{...}}`）
- `[ab]`匹配单个字符`a`或`b`
  - 使用`[0-9]`匹配数字0到1
  - 使用`[!a-zA-Z]`匹配除了`a`到`z`的小写字母和`A`到`Z`的大写字母外的字符。
  - 可通过方括号来转义元字符，如`[*]`匹配`*`、`[/]`匹配`/`。

除此以外，路径还支持两种特殊的语法：
- `// ./Samples/*.wav`：这会继续匹配磁盘上的文件，并为匹配的路径生成对应的行。但在输出时，这些行不会将opcode`sample`写入SFZ中。
- `"./Samples/*.wav"`：这会使输出的路径被包裹在一对引号中。

这两种特殊语法可以与`@header`的合并范围进行配合，但要注意，每个合并范围只能在` `、`//`、`""`中选择一种来使用，即同一个合并范围内的路径应保持一致，示例：
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

被`@raw`标注的列会直接输出单元格的内容。该注解为可选，可声明多次。
|@header|@sample|@raw|@raw|
|--|--|--|--|
|\<region\>|./Bass_k60_ampv127.wav|key=${k}|amp_velcurve_${ampv}=1|

```c
<region> sample=./Bass_k60_ampv127 key=60 amp_velcurve_127=1
```

### 单元格

非标题单元格接受任意字符串作为输入，并且支持数学表达式。
表达式可通过`${...}`来定义，除了基本的运算符`+`、`-`、`*`、`/`、`^`，还可使用下列内部函数：
- `sin(x)`, `cos(x)`, `tan(x)`, `asin(x)`, `acos(x)`, `atan(x)`：三角函数相关。
- `sqrt(x)`：`x`的正平方根。
- `log(x,a)`：以`a`为底`x`的对数。
- `abs(x)`：`x`的绝对值。
- `ceil(x)`：`x`向上取整。
- `floor(x)`：`x`向下取整。
- `round(x,n=0)`：`x`四舍五入，保留`n`位小数，默认返回整数。
- `max(a,b)`：取最大值。
- `min(a,b)`：取最小值。
- `sat(x)`：限制`x`区间为`[0,1]`。
- `vsat(x)`：限制`x`区间为`[0,127]`。
- `nl(x,k=-2)`：一个非线性函数，用于对区间为`[0,1]`的线性输入进行非线性缩放。当`k`为负时，`k`越小，输出值靠近`1`的一端分布就越稠密；`k`为正时，`k`越大，靠近`0`的一端就越稠密。参数`k`默认值为`-2`，公式为：

$$ f(x,k)=\frac{2^{k \cdot x} - 1}{2^{k} - 1} $$

表达式还可使用在`@sample`文件名中声明的参数。参数的声明格式遵循`名称` `值`，名称与值两项之间没有任何分隔符，所以参数名称仅支持大小写字母，参数值仅支持整型或浮点。声明多个参数时，不同参数之间使用`_`分隔。

例如，一个参数有效的文件名`Drum_k60_vol1.5_v1_l3.wav`，其中`k=60`，`vol=1.5`，`v=1`，`l=3`。你可以在表达式中使用这些参数，比如在`@raw`标注的列中，`amp_velcurve_${vsat(floor(nl(v/l)*127))}=1`。

## FAQ

### 行顺序
对于输出的SFZ，同一合并范围内的行是无序的，不同合并范围的行之间的相对顺序是有序的，顺序与输入一致。
### 列顺序
对于输出的SFZ，列是有序的，顺序与输入一致。
### 文件名参数
参数名仅支持字母，参数值仅支持整型或浮点。参数没有顺序要求，可以与非参数文本进行混合，只要你使用`_`进行分隔。
### 数学表达式
输入仅支持整型或浮点，在内部均按照64位浮点进行计算。无效的表达式会被直接输出至SFZ中，方便DEBUG。
### 列标题
除了特殊注解，每个列标题都应是独特的，重复的列标题会导致意外行为。在内部，程序依赖双下划线`__`来处理`@raw`等不必输出的列标题，所以列标题应避免使用双下划线`__`前缀。另外，空列标题及其列的内容不会被输出。
