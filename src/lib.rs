use derive_more::derive::From;
use globset::GlobMatcher;
use indexmap::IndexMap;
use rayon::iter::IntoParallelRefIterator;
use rayon::prelude::*;
use regex::Regex;
use std::{
    collections::HashMap,
    ffi::{CStr, c_char, c_int},
    fs::{self},
    io::Write,
    ops::{Not, Range},
    path::Path,
    sync::LazyLock,
};

/// Recursively convert any CSV file in the directory to SFZ.
///
/// # Arguments
///
/// * `dir_path` - A null-terminated C string representing the path to the directory containing
///                  samples and CSV files.
///
/// # Returns
///
/// *  `0` - Execution succeeded.
/// * `-1` - Invalid input path.
/// * `-2` - Error occurred while traversing directories.
/// * `-3` - Error occurred while parsing CSV files.
/// * `-4` - Error occurred while processing CSV expressions.
/// * `-5` - Error occurred while saving sfz files to disk.
///
/// # Safety
///
/// This function is unsafe because it dereferences a raw pointer. The caller must ensure that
/// the provided `dir_path` pointer is non-null and points to a valid, null-terminated C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn generate_sfz(dir_path: *const c_char) -> c_int {
    let Some(path) = try_get_dir_path(dir_path) else {
        return -1;
    };

    let mut sample_paths = Vec::new();
    let mut meta_paths = Vec::new();
    if traverse_directory(path, path, &mut sample_paths, &mut meta_paths).is_err() {
        return -2;
    }

    let rows_vars = sample_paths
        .iter()
        .filter_map(|sample_path| {
            let path = Path::new(sample_path);
            let sample_name = path.file_stem().and_then(|s| s.to_str())?;
            let sample = parse_sample_name(sample_name);
            Some((path.to_str()?, sample))
        })
        .collect::<HashMap<&str, HashMap<&str, &str>>>();

    let result = meta_paths.par_iter().try_for_each(|meta_path| {
        let csv_path = Path::new(meta_path);
        let Ok(mut sample_csv) = parse_sample_csv(csv_path) else {
            return Err(-3);
        };
        if expand_sample_csv(&mut sample_csv, &sample_paths, &rows_vars).is_err() {
            return Err(-4);
        };

        let sfz_path = csv_path.with_extension("sfz");
        if generate_sfz_file(sfz_path, &sample_csv).is_err() {
            return Err(-5);
        };
        Ok(())
    });

    match result {
        Ok(_) => 0,
        Err(code) => code,
    }
}

fn try_get_dir_path<'a>(dir_path: *const c_char) -> Option<&'a Path> {
    if dir_path.is_null() {
        return None;
    }

    let c_str = unsafe { CStr::from_ptr(dir_path) };
    let input_path_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return None,
    };

    let path = Path::new(input_path_str);

    if !path.is_dir() {
        return None;
    }

    Some(path)
}

#[allow(dead_code)]
#[derive(Debug, From)]
enum Error {
    #[from]
    Io(std::io::Error),
    #[from]
    StripPrefix(std::path::StripPrefixError),
    InvalidUnicode,
    #[from]
    CSVErr(csv::Error),
    CSVOpcode,
    CSVHeader,
    #[from]
    Glob(globset::Error),
}

// Give control to users
// const EXT_SAMPLE: [&str; 8] = ["wav", "flac", "ogg", "mp3", "aif", "aiff", "aifc", "wv"];
const EXT_META: [&str; 1] = ["csv"];

fn traverse_directory<P: AsRef<Path>, Q: AsRef<Path>>(
    root_path: P,
    cur_path: Q,
    sample_paths: &mut Vec<String>,
    meta_paths: &mut Vec<String>,
) -> Result<(), Error> {
    let root_path = root_path.as_ref();
    for entry in fs::read_dir(cur_path)? {
        let entry = entry?;
        let entry_path = entry.path();

        if entry_path.is_dir() {
            traverse_directory(root_path, entry_path, sample_paths, meta_paths)?;
        } else if let Some(ext) = entry_path.extension().and_then(|s| s.to_str()) {
            match ext {
                ext if EXT_META.contains(&ext) => {
                    meta_paths.push(
                        entry_path
                            .to_str()
                            .ok_or(Error::InvalidUnicode)?
                            .to_string(),
                    );
                }
                _ => {
                    let relative_path = entry_path.strip_prefix(root_path)?;
                    sample_paths.push(format!(
                        "./{}",
                        relative_path.to_str().ok_or(Error::InvalidUnicode)?
                    ));
                }
            }
        }
    }
    Ok(())
}

fn parse_opcode(param: &str) -> Option<(&str, &str)> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^([a-zA-Z]+)(-?\d+\.?\d*)$").unwrap());
    RE.captures(param)
        .and_then(|c| Some((c.get(1)?.as_str(), c.get(2)?.as_str())))
}

fn parse_sample_name(name: &str) -> HashMap<&str, &str> {
    name.split('_')
        .filter_map(|param| {
            if param.is_empty() {
                return None;
            };
            // param.split_once('=').or_else(|| parse_opcode(param))
            parse_opcode(param)
        })
        .collect()
}

/// If return Some(), the first element of Vec is promised to be valid.
fn parse_annotation(input: &str) -> Option<Vec<&str>> {
    static RE: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^@([a-zA-Z_][a-zA-Z0-9_]*)(?:\((.*?)\))?$").unwrap());

    let captures = RE.captures(input)?;
    let mut result = Vec::new();

    result.push(captures.get(1)?.as_str());

    if let Some(params) = captures.get(2) {
        let params_list = params
            .as_str()
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .collect::<Vec<&str>>();
        result.extend(params_list);
    }

    Some(result)
}

#[derive(Debug, Clone)]
struct SampleCSV {
    opcode_indices: IndexMap<String, usize>, // Used for output
    anno_indices: HashMap<String, usize>,    // Used for find annotations
    header_ranges: Vec<Range<usize>>,
    rows: Vec<Vec<String>>,
}

fn parse_sample_csv(path: impl AsRef<Path>) -> Result<SampleCSV, Error> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_path(path.as_ref())?;

    let mut records = reader.records();

    let opcodes = records
        .next()
        .and_then(|record| record.ok())
        .ok_or(Error::CSVOpcode)?;

    let mut anno_indices = HashMap::new();
    let mut opcode_indices = IndexMap::new();
    create_indices(opcodes, &mut opcode_indices, &mut anno_indices);

    let rows: Vec<Vec<String>> = records
        .filter_map(|record| {
            let rec = record.ok()?;
            Some(rec.iter().map(|s| s.to_string()).collect())
        })
        .collect();

    let header_idx = anno_indices.get("header").ok_or(Error::CSVHeader)?;
    let mut header_ranges = Vec::new();
    creat_header_ranges(&rows, &mut header_ranges, header_idx);

    Ok(SampleCSV {
        opcode_indices,
        anno_indices,
        header_ranges,
        rows,
    })
}

fn create_indices(
    opcodes: csv::StringRecord,
    opcode_indices: &mut IndexMap<String, usize>,
    anno_indices: &mut HashMap<String, usize>,
) {
    *opcode_indices = opcodes
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let a = parse_annotation(a)
                .map(|anno| match anno[0] {
                    "raw" => {
                        anno_indices.insert(anno[0].to_string(), i);
                        format!("__raw_{}", i)
                    }
                    "sample" => {
                        anno_indices.insert(anno[0].to_string(), i);
                        anno.get(1).unwrap_or(&"sample").to_string()
                    }
                    "header" => {
                        anno_indices.insert(anno[0].to_string(), i);
                        String::from("__header")
                    }
                    _ => a.to_string(),
                })
                .unwrap_or(a.to_string());
            (a, i)
        })
        .collect::<IndexMap<String, usize>>();
}

fn creat_header_ranges(
    rows: &[Vec<String>],
    header_ranges: &mut Vec<Range<usize>>,
    header_idx: &usize,
) {
    let mut start = 0;
    for (i, row) in rows.iter().enumerate().skip(1) {
        if !row[*header_idx].is_empty() {
            header_ranges.push(start..i);
            start = i;
        }
    }
    if !rows.is_empty() {
        header_ranges.push(start..rows.len());
    }
}

fn apply_expr(cell: &mut String, ctx: Option<mexprp::Context<f64>>) -> Result<(), Error> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\$\{([^}]+)\}").unwrap());
    *cell = RE
        .replace_all(cell, |caps: &regex::Captures| {
            caps.get(1)
                .and_then(|m| {
                    mexprp::Expression::parse_ctx(m.as_str(), ctx.clone()?)
                        .ok()?
                        .eval()
                        .and_then(math_expr::format_float)
                        .ok()
                })
                .unwrap_or_default()
        })
        .to_string();
    Ok(())
}

mod math_expr {
    use mexprp::*;

    pub(crate) fn format_float(val: Answer<f64>) -> Result<String, MathError> {
        let v = match val {
            Answer::Single(v) => v,
            Answer::Multiple(v) => v[0],
        };
        Ok((if v == -0.0 { 0.0 } else { v }).to_string())
    }

    fn py_round(n: &f64, decimals: &f64) -> f64 {
        let factor = 10f64.powi(*decimals as i32);
        (n * factor).round() / factor
    }

    type Exp = fn(&[Term<f64>], &Context<f64>) -> Calculation<f64>;
    pub(crate) const EXPS: [(&str, Exp); 5] = [
        (
            "ceil",
            |args: &[Term<f64>], ctx: &Context<f64>| -> Calculation<f64> {
                type E = MathError;
                if args.len() != 1 {
                    return Err(E::IncorrectArguments);
                }
                let a = args.first().ok_or(E::IncorrectArguments)?.eval_ctx(ctx)?;
                let b = Answer::Single(0.0);
                a.op(&b, |a, _| Num::ceil(a, ctx))
            },
        ),
        (
            "round",
            |args: &[Term<f64>], ctx: &Context<f64>| -> Calculation<f64> {
                type E = MathError;
                if args.is_empty() || args.len() > 2 {
                    return Err(E::IncorrectArguments);
                }
                let a = args.first().ok_or(E::IncorrectArguments)?.eval_ctx(ctx)?;
                let b = args
                    .get(1)
                    .ok_or(E::IncorrectArguments)
                    .and_then(|b| b.eval_ctx(ctx))
                    .unwrap_or(Answer::Single(0.0));
                a.op(&b, |a, b| Num::from_f64(py_round(a, b), ctx))
            },
        ),
        (
            "sat",
            |args: &[Term<f64>], ctx: &Context<f64>| -> Calculation<f64> {
                type E = MathError;
                if args.len() != 1 {
                    return Err(E::IncorrectArguments);
                }
                let a = args.first().ok_or(E::IncorrectArguments)?.eval_ctx(ctx)?;
                let b = Answer::Single(0.0);
                a.op(&b, |a, _| Num::from_f64(a.clamp(0., 1.).abs(), ctx))
            },
        ),
        (
            "vsat",
            |args: &[Term<f64>], ctx: &Context<f64>| -> Calculation<f64> {
                type E = MathError;
                if args.len() != 1 {
                    return Err(E::IncorrectArguments);
                }
                let a = args.first().ok_or(E::IncorrectArguments)?.eval_ctx(ctx)?;
                let b = Answer::Single(0.0);
                a.op(&b, |a, _| Num::from_f64(a.clamp(0., 127.).abs(), ctx))
            },
        ),
        (
            "nl",
            |args: &[Term<f64>], ctx: &Context<f64>| -> Calculation<f64> {
                type E = MathError;
                if args.is_empty() || args.len() > 2 {
                    return Err(E::IncorrectArguments);
                }
                let a = args.first().ok_or(E::IncorrectArguments)?.eval_ctx(ctx)?;
                let b = args
                    .get(1)
                    .ok_or(E::IncorrectArguments)
                    .and_then(|b| b.eval_ctx(ctx))
                    .unwrap_or(Answer::Single(-2.0));
                a.op(&b, |a, b| {
                    Num::from_f64(((b * a).exp2() - 1.0) / (b.exp2() - 1.0), ctx)
                })
            },
        ),
    ];
}

fn map_to_ctx(row_vars: Option<&HashMap<&str, &str>>) -> Option<mexprp::Context<f64>> {
    row_vars.map(|row_vars| {
        let mut ctx = mexprp::Context::<f64>::new();
        math_expr::EXPS
            .iter()
            .for_each(|(name, func)| ctx.set_func(name, func));
        row_vars.iter().fold(ctx, |mut ctx, (k, v)| {
            if let Ok(v) = v.parse::<f64>() {
                ctx.set_var(k, v)
            }
            ctx
        })
    })
}

fn merge_row(
    new_row: &mut [String],
    old_row: &mut [String],
    row_vars: Option<&HashMap<&str, &str>>,
) {
    let ctx = map_to_ctx(row_vars);
    new_row
        .iter_mut()
        .zip(old_row.iter_mut())
        .for_each(|(new_cell, old_cell)| {
            if let Some(_vars) = row_vars {
                let _ = apply_expr(new_cell, ctx.clone());
            }
            if !new_cell.is_empty() {
                *old_cell = new_cell.clone();
            }
        });
}

fn insert_row(mut new_row: Vec<String>, rows_vars: Option<&HashMap<&str, &str>>) -> Vec<String> {
    if let Some(row_vars) = rows_vars {
        let ctx = map_to_ctx(Some(row_vars));
        new_row.iter_mut().for_each(|cell| {
            let _ = apply_expr(cell, ctx.clone());
        });
    }
    new_row
}

fn trim_pair(input: &str) -> Option<&str> {
    if input.starts_with('"') && input.ends_with('"') {
        Some(&input[1..input.len() - 1])
    } else {
        None
    }
}

fn trim_comment_prefix(input: &str) -> Option<&str> {
    input
        .starts_with("//")
        .then(|| input.trim_start_matches("//").trim_start())
}

fn try_get_matcher(pattern: &str) -> Result<GlobMatcher, Error> {
    let glob_builder = globset::GlobBuilder::new(pattern)
        .literal_separator(true)
        .build()?;
    Ok(glob_builder.compile_matcher())
}

fn matching_paths(
    sample_paths: &[String],
    sample_idx: usize,
    row: &[String],
    matcher: GlobMatcher,
    path_modifier: fn(&mut String),
) -> HashMap<String, Vec<String>> {
    sample_paths
        .iter()
        .filter(|path| matcher.is_match(path))
        .map(|path| {
            let mut row: Vec<String> = row.to_vec();
            row[sample_idx] = path.clone();
            path_modifier(&mut row[sample_idx]);
            (path.clone(), row)
        })
        .collect()
}

fn expand_sheet(
    rows: &[Vec<String>],
    sample_paths: &[String],
    rows_vars: &HashMap<&str, HashMap<&str, &str>>,
    sample_idx: Option<&usize>,
) -> Result<Vec<Vec<String>>, Error> {
    sample_idx
        .and_then(|sample_idx| {
            const PATH_MODIFIER_PASS: fn(&mut String) = |_| {};
            const PATH_MODIFIER_CLEAR: fn(&mut String) = |input| input.clear();
            const PATH_MODIFIER_PAIR: fn(&mut String) = |input| *input = format!("\"{}\"", input);
            let first_path = &rows.first()?[*sample_idx];
            first_path.is_empty().not().then(|| {
                let sample_path = first_path.as_str();
                let path_modifier = match trim_comment_prefix(sample_path) {
                    Some(_) => PATH_MODIFIER_CLEAR,
                    None => match trim_pair(sample_path) {
                        Some(_) => PATH_MODIFIER_PAIR,
                        None => PATH_MODIFIER_PASS,
                    },
                };
                (sample_idx, path_modifier)
            })
        })
        .map_or_else(
            || Ok::<Vec<Vec<String>>, Error>(rows.to_vec()),
            |(&sample_idx, path_modifier)| {
                let r = rows
                    .par_iter()
                    .map(|row| {
                        let sample_path = row[sample_idx].as_str();
                        let sample_path = trim_comment_prefix(sample_path)
                            .or(Some(sample_path))
                            .and_then(trim_pair)
                            .unwrap_or(sample_path);
                        let r = try_get_matcher(sample_path).map(|matcher| {
                            matching_paths(sample_paths, sample_idx, row, matcher, path_modifier)
                        })?;
                        Ok::<HashMap<String, Vec<String>>, Error>(r)
                    })
                    .try_reduce(HashMap::new, |mut acc, unfolded_rows| {
                        unfolded_rows.into_iter().for_each(|(key, mut new_row)| {
                            acc.entry(key.clone())
                                .and_modify(|old_row| {
                                    merge_row(&mut new_row, old_row, rows_vars.get(key.as_str()))
                                })
                                .or_insert_with(|| {
                                    insert_row(new_row, rows_vars.get(key.as_str()))
                                });
                        });
                        Ok(acc)
                    })?
                    .into_values()
                    .collect();
                Ok(r)
            },
        )
}

fn expand_sample_csv(
    sample_csv: &mut SampleCSV,
    sample_paths: &[String],
    rows_vars: &HashMap<&str, HashMap<&str, &str>>,
) -> Result<(), Error> {
    let sample_idx = sample_csv.anno_indices.get("sample");

    sample_csv.rows = sample_csv
        .header_ranges
        .clone()
        .into_par_iter()
        .map(|range| {
            let rows = &sample_csv.rows[range];
            expand_sheet(rows, sample_paths, rows_vars, sample_idx)
        })
        .try_reduce(Vec::new, |mut acc, partial| {
            acc.extend(partial);
            Ok(acc)
        })?;

    Ok(())
}

fn generate_sfz_file(path: impl AsRef<Path>, sample_csv: &SampleCSV) -> Result<(), Error> {
    let mut sfz: String = String::new();
    for row in sample_csv.rows.iter() {
        sample_csv
            .opcode_indices
            .iter()
            .filter(|(key, _)| !key.is_empty())
            .for_each(|(key, idx)| {
                row.get(*idx).and_then(|value| {
                    value.is_empty().not().then(|| {
                        if key.starts_with("__") {
                            sfz.push_str(&format!("{} ", value));
                        } else {
                            sfz.push_str(&format!("{}={} ", key, value));
                        }
                    })
                });
            });
        sfz.push('\n');
    }
    let mut file = fs::File::create(path)?;
    file.write_all(sfz.as_bytes())?;
    Ok(())
}

#[cfg(test)]
mod tests {

    use super::*;

    #[test]
    fn test_sample_name_paser() {
        let name = "$0m@_(#)*_kEy60_.str99_pi3.14_zdot0._dotz.0_ddd1.2.3";
        let result = parse_sample_name(name);

        assert_eq!(result.get("kEy"), Some(&"60"));
        assert_eq!(result.get(".str"), None);
        assert_eq!(result.get("str"), None);
        assert_eq!(result.get("pi"), Some(&"3.14"));
        assert_eq!(result.get("zdot"), Some(&"0."));
        assert_eq!(result.get("dotz"), None);
        assert_eq!(result.get("dotz."), None);
        assert_eq!(result.get("ddd"), None);
    }

    #[test]
    fn test_create_indices() {
        let opcodes =
            csv::StringRecord::from(vec!["@raw".to_string(), "@sample(path)".to_string()]);
        let mut opcode_indices = IndexMap::new();
        let mut anno_indices = HashMap::new();
        create_indices(opcodes, &mut opcode_indices, &mut anno_indices);

        assert_eq!(opcode_indices.len(), 2);
        assert_eq!(anno_indices.len(), 2);
        assert_eq!(opcode_indices.get("_raw_0"), Some(&0));
        assert_eq!(opcode_indices.get("path"), Some(&1));
        assert_eq!(anno_indices.get("raw"), Some(&0));
        assert_eq!(anno_indices.get("sample"), Some(&1));
    }

    #[test]
    fn test_creat_header_ranges() {
        let rows: Vec<Vec<String>> = vec![
            vec!["".to_string(), "b".to_string()],
            vec!["".to_string(), "d".to_string()],
            vec!["c".to_string(), "d".to_string()],
        ];
        let mut header_ranges = Vec::new();
        let header_idx = 0;
        creat_header_ranges(&rows, &mut header_ranges, &header_idx);

        assert_eq!(header_ranges, vec![0..2, 2..3]);
    }

    #[test]
    fn test_glob() {
        let glob = globset::Glob::new("").unwrap();
        let matcher = glob.compile_matcher();

        assert!(!matcher.is_match("test"));
        assert!(matcher.is_match(""));
    }

    #[test]
    fn test_mexprp() {
        let mut cells: Vec<String> = [
            "${2^2}",
            "${sqrt(49)}",  // Pick First
            "${nrt(4, 2)}", // NOT WORK
            "${abs(-4)}",
            "${round(sin(3.14))}",
            "${round(cos(pi))}",
            "${tan(0)}",
            "${asin(0)}",
            "${acos(1)}",
            "${atan(0)}",
            "${atan2(0,0)}", // NOT WORK
            "${floor(0.5)}",
            "${ceil(1.5)}", // NOT WORK, but fixed manually
            "${round(3.14155, 3)}",
            "${log(1, 2)}",
            "${sat(2)}",
            "${vsat(200)}",
            "${round(nl(0.5, -2), 2)}",
            "${max(0.5, -2)}",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let answers: Vec<String> = [
            "4", "7", "", "4", "0", "-1", "0", "0", "0", "0", "", "0", "2", "3.142", "0", "1",
            "127", "0.67", "0.5",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let row_vars = vec![("l", "3"), ("v", "2")]
            .into_iter()
            .collect::<HashMap<&str, &str>>();

        let ctx = map_to_ctx(Some(&row_vars)).unwrap();

        for (cell, answer) in cells.iter_mut().zip(answers.iter()) {
            let e = apply_expr(cell, Some(ctx.clone()));
            assert!(e.is_ok());
            assert_eq!(cell, answer);
        }
    }

    #[test]
    fn test_apply_expr() {
        let mut cell = "This is ${v/l*127}.".to_string();
        let row_vars = vec![("l", "3"), ("v", "2")]
            .into_iter()
            .collect::<HashMap<&str, &str>>();

        let ctx = map_to_ctx(Some(&row_vars)).unwrap();
        let e = apply_expr(&mut cell, Some(ctx));

        assert!(e.is_ok());
        assert_eq!(cell, format!("This is {:.2}.", 2. / 3. * 127.));
    }

    #[test]
    fn test_expand_sample_csv() {
        macro_rules! vec_str {
            ($($s:expr),*) => (vec![$($s.to_string()),*]);
        }

        let mut sample_csv = SampleCSV {
            opcode_indices: vec![("key", 0), ("sample", 1), ("_header", 3)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            rows: vec![
                vec_str!["${k}", "./path/to/*.wav", "<regionA>"],
                vec_str!["-1", "./path/to/sample1.wav", ""],
                vec_str!["${k+v}", "./path/to/*{1,3,5}.wav", "<regionB>"],
            ],
            anno_indices: vec![("sample", 1), ("header", 3)]
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            header_ranges: vec![0..2, 2..3],
        };

        let sample_paths = (1..=5)
            .map(|i| format!("./path/to/sample{i}.wav"))
            .collect::<Vec<_>>();

        let vars = vec![
            ("./path/to/sample1.wav", vec![("k", "1"), ("v", "100")]),
            ("./path/to/sample2.wav", vec![("k", "2"), ("v", "100")]),
            ("./path/to/sample3.wav", vec![("k", "3"), ("v", "100")]),
            ("./path/to/sample4.wav", vec![("k", "4"), ("v", "100")]),
            ("./path/to/sample5.wav", vec![("k", "5"), ("v", "100")]),
        ];
        let rows_vars: HashMap<&str, HashMap<&str, &str>> = vars
            .into_iter()
            .map(|(path, vars)| (path, vars.into_iter().collect()))
            .collect();

        expand_sample_csv(&mut sample_csv, &sample_paths, &rows_vars).unwrap();

        assert_eq!(sample_csv.rows.len(), 8);

        let expected_rows = [
            ["-1", "./path/to/sample1.wav", "<regionA>"],
            ["2", "./path/to/sample2.wav", "<regionA>"],
            ["3", "./path/to/sample3.wav", "<regionA>"],
            ["4", "./path/to/sample4.wav", "<regionA>"],
            ["5", "./path/to/sample5.wav", "<regionA>"],
            ["101", "./path/to/sample1.wav", "<regionB>"],
            ["103", "./path/to/sample3.wav", "<regionB>"],
            ["105", "./path/to/sample5.wav", "<regionB>"],
        ]
        .map(|arr| arr.map(|s| s.to_string()).to_vec())
        .into_iter()
        .collect::<std::collections::HashSet<_>>();

        let actual_rows = sample_csv
            .rows
            .into_iter()
            .collect::<std::collections::HashSet<_>>();

        assert_eq!(actual_rows, expected_rows);
    }
}
