use regex::{NoExpand, Regex, RegexBuilder};
use std::cmp::Ordering;
use std::fs;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RenameMode {
    Replace,
    Regex,
    Numbering,
    Date,
    Affix,
}

impl RenameMode {
    pub const ALL: [RenameMode; 5] = [
        RenameMode::Replace,
        RenameMode::Regex,
        RenameMode::Numbering,
        RenameMode::Date,
        RenameMode::Affix,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RenameMode::Replace => "字串替换",
            RenameMode::Regex => "正则表达式",
            RenameMode::Numbering => "自动编号",
            RenameMode::Date => "添加日期",
            RenameMode::Affix => "前缀 / 后缀",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            RenameMode::Replace => "替换",
            RenameMode::Regex => "正则",
            RenameMode::Numbering => "编号",
            RenameMode::Date => "日期",
            RenameMode::Affix => "前后缀",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DateFormat {
    Dashed,
    Compact,
}

impl DateFormat {
    pub const ALL: [DateFormat; 2] = [DateFormat::Dashed, DateFormat::Compact];

    pub fn label(self) -> &'static str {
        match self {
            DateFormat::Dashed => "yyyy-mm-dd",
            DateFormat::Compact => "yyyymmdd",
        }
    }

    fn render(self, year: u16, month: u16, day: u16) -> String {
        match self {
            DateFormat::Dashed => format!("{year:04}-{month:02}-{day:02}"),
            DateFormat::Compact => format!("{year:04}{month:02}{day:02}"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatePosition {
    Prefix,
    Suffix,
}

impl DatePosition {
    pub const ALL: [DatePosition; 2] = [DatePosition::Prefix, DatePosition::Suffix];

    pub fn label(self) -> &'static str {
        match self {
            DatePosition::Prefix => "前缀",
            DatePosition::Suffix => "后缀",
        }
    }
}

#[derive(Clone, Debug)]
pub struct RenameRule {
    pub mode: RenameMode,
    pub find: String,
    pub replace: String,
    pub ignore_case: bool,
    pub keep_extension: bool,
    pub template: String,
    pub start: u64,
    pub step: u64,
    pub digits: u32,
    pub prefix: String,
    pub suffix: String,
    pub date_format: DateFormat,
    pub date_position: DatePosition,
}

impl Default for RenameRule {
    fn default() -> Self {
        Self {
            mode: RenameMode::Replace,
            find: String::new(),
            replace: String::new(),
            ignore_case: false,
            keep_extension: true,
            template: "{name}_{n}".to_owned(),
            start: 1,
            step: 1,
            digits: 3,
            prefix: String::new(),
            suffix: String::new(),
            date_format: DateFormat::Dashed,
            date_position: DatePosition::Prefix,
        }
    }
}

#[derive(Clone, Debug)]
pub struct PlannedRename {
    pub old_name: String,
    pub new_name: String,
    pub is_dir: bool,
    pub error: Option<String>,
    pub changed: bool,
}

impl PlannedRename {
    pub fn is_renameable(&self) -> bool {
        self.changed && self.error.is_none()
    }
}

#[derive(Clone, Debug, Default)]
pub struct RenamePlan {
    pub rows: Vec<PlannedRename>,
    pub changed_count: usize,
    pub error_count: usize,
}

impl RenamePlan {
    pub fn can_apply(&self) -> bool {
        self.changed_count > 0 && self.error_count == 0
    }
}

#[derive(Clone, Debug, Default)]
pub struct ApplyReport {
    pub renamed: usize,
    pub failures: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Clone, Debug)]
pub struct RenameFilter {
    pub files: bool,
    pub dirs: bool,
    pub extensions: Option<Vec<String>>,
}

impl Default for RenameFilter {
    fn default() -> Self {
        Self {
            files: true,
            dirs: false,
            extensions: None,
        }
    }
}

impl RenameFilter {
    fn accepts(&self, entry: &Entry) -> bool {
        if entry.is_dir {
            self.dirs
        } else if !self.files {
            false
        } else {
            match &self.extensions {
                Some(extensions) => extensions
                    .iter()
                    .any(|extension| extension == &extension_key(&entry.name)),
                None => true,
            }
        }
    }
}

pub fn extension_key(name: &str) -> String {
    split_name(name)
        .1
        .map(|extension| extension.to_lowercase())
        .unwrap_or_default()
}

pub fn list_entries(dir: &Path, skipped_non_utf8: &mut usize) -> Result<Vec<Entry>, String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("无法读取文件夹：{e}"))?;
    let mut items = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let is_dir = file_type.is_dir();
        if !is_dir && !file_type.is_file() {
            continue;
        }
        match entry.file_name().into_string() {
            Ok(name) => items.push(Entry { name, is_dir }),
            Err(_) => *skipped_non_utf8 += 1,
        }
    }
    items.sort_by(|a, b| natural_cmp(&a.name, &b.name));
    Ok(items)
}

enum NameOperation {
    Replace { find: String, replace: String, regex: Option<Regex> },
    Regex { regex: Regex, replace: String },
    Numbering { template: String, start: u64, step: u64, digits: u32 },
    Date { stamp: String, position: DatePosition },
    Affix { prefix: String, suffix: String },
}

impl NameOperation {
    fn new(rule: &RenameRule) -> Result<Self, String> {
        match rule.mode {
            RenameMode::Replace => {
                if rule.find.is_empty() {
                    return Err("请填写要查找的字串。".to_owned());
                }
                let regex = if rule.ignore_case {
                    Some(
                        RegexBuilder::new(&regex::escape(&rule.find))
                            .case_insensitive(true)
                            .build()
                            .map_err(|e| format!("字串规则无效：{e}"))?,
                    )
                } else {
                    None
                };
                Ok(NameOperation::Replace {
                    find: rule.find.clone(),
                    replace: rule.replace.clone(),
                    regex,
                })
            }
            RenameMode::Regex => {
                if rule.find.is_empty() {
                    return Err("请填写正则表达式。".to_owned());
                }
                let regex = RegexBuilder::new(&rule.find)
                    .case_insensitive(rule.ignore_case)
                    .build()
                    .map_err(|e| format!("正则表达式无效：{e}"))?;
                Ok(NameOperation::Regex {
                    regex,
                    replace: rule.replace.clone(),
                })
            }
            RenameMode::Numbering => {
                if !rule.template.contains("{n}") {
                    return Err("编号模板中必须包含 {n}。".to_owned());
                }
                Ok(NameOperation::Numbering {
                    template: rule.template.clone(),
                    start: rule.start,
                    step: rule.step,
                    digits: rule.digits.min(12),
                })
            }
            RenameMode::Date => {
                let time = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
                Ok(NameOperation::Date {
                    stamp: rule.date_format.render(time.wYear, time.wMonth, time.wDay),
                    position: rule.date_position,
                })
            }
            RenameMode::Affix => Ok(NameOperation::Affix {
                prefix: rule.prefix.clone(),
                suffix: rule.suffix.clone(),
            }),
        }
    }

    fn apply_to_stem(&self, stem: &str, index: usize) -> Result<String, String> {
        match self {
            NameOperation::Replace {
                find,
                replace,
                regex,
            } => Ok(match regex {
                Some(regex) => regex
                    .replace_all(stem, NoExpand(replace.as_str()))
                    .into_owned(),
                None => stem.replace(find.as_str(), replace),
            }),
            NameOperation::Regex { regex, replace } => {
                Ok(regex.replace_all(stem, replace.as_str()).into_owned())
            }
            NameOperation::Numbering {
                template,
                start,
                step,
                digits,
            } => {
                let index = u64::try_from(index).map_err(|_| "文件数量过多。")?;
                let number = start
                    .checked_add(index.checked_mul(*step).ok_or("编号超出范围。")?)
                    .ok_or("编号超出范围。")?;
                let formatted = format!("{number:0width$}", width = *digits as usize);
                Ok(template
                    .replace("{name}", stem)
                    .replace("{n}", &formatted))
            }
            NameOperation::Date { stamp, position } => Ok(match position {
                DatePosition::Prefix => format!("{stamp}_{stem}"),
                DatePosition::Suffix => format!("{stem}_{stamp}"),
            }),
            NameOperation::Affix { prefix, suffix } => Ok(format!("{prefix}{stem}{suffix}")),
        }
    }
}

pub fn make_plan(
    dir: &Path,
    entries: &[Entry],
    filter: &RenameFilter,
    rule: &RenameRule,
) -> Result<RenamePlan, String> {
    if !dir.is_dir() {
        return Err("目标文件夹不存在。".to_owned());
    }
    let operation = NameOperation::new(rule)?;
    let preserve_extension = matches!(rule.mode, RenameMode::Replace | RenameMode::Regex)
        && rule.keep_extension;
    let always_preserve = matches!(
        rule.mode,
        RenameMode::Numbering | RenameMode::Date | RenameMode::Affix
    );

    let mut rows: Vec<PlannedRename> = Vec::with_capacity(entries.len());
    let mut index = 0usize;
    for entry in entries.iter().filter(|entry| filter.accepts(entry)) {
        let old_name = entry.name.as_str();
        let result = if entry.is_dir {
            operation.apply_to_stem(old_name, index)
        } else if preserve_extension || always_preserve {
            let (stem, extension) = split_name(old_name);
            operation
                .apply_to_stem(stem, index)
                .map(|new_stem| match extension {
                    Some(extension) => format!("{new_stem}.{extension}"),
                    None => new_stem,
                })
        } else {
            operation.apply_to_stem(old_name, index)
        };
        index += 1;

        match result {
            Ok(new_name) => {
                let changed = new_name != old_name;
                let error = if changed {
                    validate_filename(&new_name)
                } else {
                    None
                };
                rows.push(PlannedRename {
                    old_name: old_name.to_owned(),
                    new_name,
                    is_dir: entry.is_dir,
                    error,
                    changed,
                });
            }
            Err(error) => rows.push(PlannedRename {
                old_name: old_name.to_owned(),
                new_name: String::new(),
                is_dir: entry.is_dir,
                error: Some(error),
                changed: false,
            }),
        }
    }

    mark_conflicts(dir, &mut rows);

    let changed_count = rows.iter().filter(|row| row.changed).count();
    let error_count = rows.iter().filter(|row| row.error.is_some()).count();
    Ok(RenamePlan {
        rows,
        changed_count,
        error_count,
    })
}

fn listing_plan(entries: &[Entry], filter: &RenameFilter) -> RenamePlan {
    let rows = entries
        .iter()
        .filter(|entry| filter.accepts(entry))
        .map(|entry| PlannedRename {
            old_name: entry.name.clone(),
            new_name: entry.name.clone(),
            is_dir: entry.is_dir,
            error: None,
            changed: false,
        })
        .collect();
    RenamePlan {
        rows,
        changed_count: 0,
        error_count: 0,
    }
}

pub fn make_preview_plan(
    dir: &Path,
    entries: &[Entry],
    filter: &RenameFilter,
    rule: &RenameRule,
) -> (RenamePlan, Option<String>) {
    match make_plan(dir, entries, filter, rule) {
        Ok(plan) => (plan, None),
        Err(error) => (listing_plan(entries, filter), Some(error)),
    }
}

pub fn apply_plan(dir: &Path, plan: &RenamePlan) -> ApplyReport {
    let mut report = ApplyReport::default();
    for row in &plan.rows {
        if !row.is_renameable() {
            continue;
        }
        match fs::rename(dir.join(&row.old_name), dir.join(&row.new_name)) {
            Ok(()) => report.renamed += 1,
            Err(error) => report
                .failures
                .push(format!("{} → {}：{error}", row.old_name, row.new_name)),
        }
    }
    report
}

fn mark_conflicts(dir: &Path, rows: &mut [PlannedRename]) {
    use std::collections::HashMap;

    let mut targets: HashMap<String, Vec<usize>> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        if row.changed && row.error.is_none() {
            targets
                .entry(row.new_name.to_lowercase())
                .or_default()
                .push(index);
        }
    }

    for indices in targets.values() {
        if indices.len() > 1 {
            for &index in indices {
                set_error(&mut rows[index], "多个项目的新名称相同");
            }
        }
    }

    for row in rows.iter_mut() {
        if !row.changed || row.error.is_some() {
            continue;
        }
        let case_only_change = row.new_name.to_lowercase() == row.old_name.to_lowercase();
        if case_only_change {
            continue;
        }
        if fs::symlink_metadata(dir.join(&row.new_name)).is_ok() {
            set_error(row, "目标名称已存在");
        }
    }
}

fn set_error(row: &mut PlannedRename, message: &str) {
    row.error = Some(match row.error.take() {
        Some(existing) => format!("{existing}；{message}"),
        None => message.to_owned(),
    });
}

fn split_name(name: &str) -> (&str, Option<&str>) {
    match name.rfind('.') {
        Some(index) if index > 0 && index + 1 < name.len() => (&name[..index], Some(&name[index + 1..])),
        _ => (name, None),
    }
}

fn validate_filename(name: &str) -> Option<String> {
    if name.is_empty() {
        return Some("新名称为空".to_owned());
    }
    if name == "." || name == ".." {
        return Some("“.” 和 “..” 不能作为文件名".to_owned());
    }
    if name.chars().any(|ch| ch < '\u{20}' || "\\/:*?\"<>|".contains(ch)) {
        return Some("包含 Windows 不允许的字符（\\ / : * ? \" < > |）".to_owned());
    }
    if name.ends_with(' ') || name.ends_with('.') {
        return Some("名称不能以空格或句点结尾".to_owned());
    }
    if name.encode_utf16().count() > 255 {
        return Some("名称超过 255 个字符".to_owned());
    }
    let device_stem = name.split('.').next().unwrap_or(name).trim_end();
    let upper = device_stem.to_ascii_uppercase();
    let reserved = matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((upper.starts_with("COM") || upper.starts_with("LPT"))
            && upper.len() == 4
            && upper.as_bytes()[3].is_ascii_digit()
            && upper.as_bytes()[3] != b'0');
    if reserved {
        return Some(format!("“{device_stem}”是 Windows 保留名称"));
    }
    None
}

pub fn natural_cmp(a: &str, b: &str) -> Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();
    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return a.cmp(b),
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let left_digits = take_digits(&mut left);
                let right_digits = take_digits(&mut right);
                let left_significant = left_digits.trim_start_matches('0');
                let right_significant = right_digits.trim_start_matches('0');
                let order = left_significant
                    .len()
                    .cmp(&right_significant.len())
                    .then_with(|| left_significant.cmp(right_significant))
                    .then_with(|| left_digits.len().cmp(&right_digits.len()));
                if order != Ordering::Equal {
                    return order;
                }
            }
            (Some(x), Some(y)) => {
                left.next();
                right.next();
                let order = x
                    .to_lowercase()
                    .cmp(y.to_lowercase())
                    .then_with(|| x.cmp(&y));
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
    }
}

fn take_digits(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) -> String {
    let mut digits = String::new();
    while let Some(ch) = chars.peek().copied() {
        if !ch.is_ascii_digit() {
            break;
        }
        digits.push(ch);
        chars.next();
    }
    digits
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use std::path::PathBuf;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "batch-renamer-test-{}-{tag}-{}",
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn touch(&self, name: &str) {
            fs::write(self.0.join(name), b"x").unwrap();
        }

        fn mkdir(&self, name: &str) {
            fs::create_dir(self.0.join(name)).unwrap();
        }

        fn entries(&self) -> HashSet<String> {
            fs::read_dir(&self.0)
                .unwrap()
                .map(|entry| entry.unwrap().file_name().into_string().unwrap())
                .collect()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn plan_for(dir: &TempDir, files: &[&str], rule: &RenameRule) -> RenamePlan {
        for name in files {
            dir.touch(name);
        }
        let entries = file_entries(files);
        make_plan(&dir.0, &entries, &RenameFilter::default(), rule).unwrap()
    }

    fn file_entries(files: &[&str]) -> Vec<Entry> {
        files
            .iter()
            .map(|name| Entry {
                name: (*name).to_owned(),
                is_dir: false,
            })
            .collect()
    }

    #[test]
    fn replace_keeps_extension() {
        let dir = TempDir::new("replace");
        let rule = RenameRule {
            mode: RenameMode::Replace,
            find: "photo".into(),
            replace: "img".into(),
            ..Default::default()
        };
        let plan = plan_for(&dir, &["photo 1.JPG", "photo 2.jpg", "notes.txt"], &rule);
        assert_eq!(plan.rows[0].new_name, "img 1.JPG");
        assert_eq!(plan.rows[1].new_name, "img 2.jpg");
        assert!(!plan.rows[2].changed);
        assert!(plan.can_apply());
    }

    #[test]
    fn replace_ignore_case() {
        let dir = TempDir::new("replace-ci");
        let rule = RenameRule {
            mode: RenameMode::Replace,
            find: "aBc".into(),
            replace: "x".into(),
            ignore_case: true,
            ..Default::default()
        };
        let plan = plan_for(&dir, &["ABC-file.txt"], &rule);
        assert_eq!(plan.rows[0].new_name, "x-file.txt");
    }

    #[test]
    fn regex_capture_groups() {
        let dir = TempDir::new("regex");
        let rule = RenameRule {
            mode: RenameMode::Regex,
            find: r"^(\d{4})-(\d{2})-(.+)$".into(),
            replace: "$3 ($1.$2)".into(),
            ..Default::default()
        };
        let plan = plan_for(&dir, &["2024-01-holiday.md"], &rule);
        assert_eq!(plan.rows[0].new_name, "holiday (2024.01).md");
    }

    #[test]
    fn numbering_uses_natural_order_and_preserves_extension() {
        let dir = TempDir::new("numbering");
        let mut skipped = 0;
        for name in ["photo10.png", "photo2.png", "photo1.png"] {
            dir.touch(name);
        }
        let files = list_entries(&dir.0, &mut skipped).unwrap();
        let rule = RenameRule {
            mode: RenameMode::Numbering,
            template: "照片_{n}".into(),
            start: 5,
            step: 2,
            digits: 3,
            ..Default::default()
        };
        let plan = make_plan(&dir.0, &files, &RenameFilter::default(), &rule).unwrap();
        assert_eq!(plan.rows[0].new_name, "照片_005.png");
        assert_eq!(plan.rows[1].new_name, "照片_007.png");
        assert_eq!(plan.rows[2].new_name, "照片_009.png");
        assert_eq!(plan.rows[0].old_name, "photo1.png");
        assert_eq!(plan.rows[1].old_name, "photo2.png");
        assert_eq!(plan.rows[2].old_name, "photo10.png");
    }

    #[test]
    fn numbering_requires_token() {
        let dir = TempDir::new("numbering-token");
        dir.touch("a.txt");
        let rule = RenameRule {
            mode: RenameMode::Numbering,
            template: "文件".into(),
            ..Default::default()
        };
        assert!(make_plan(
            &dir.0,
            &file_entries(&["a.txt"]),
            &RenameFilter::default(),
            &rule
        )
        .is_err());
    }

    #[test]
    fn prefix_suffix_before_extension() {
        let dir = TempDir::new("affix");
        let rule = RenameRule {
            mode: RenameMode::Affix,
            prefix: "老".into(),
            suffix: "-备份".into(),
            ..Default::default()
        };
        let plan = plan_for(&dir, &["照片.png"], &rule);
        assert_eq!(plan.rows[0].new_name, "老照片-备份.png");
    }

    #[test]
    fn date_mode_supports_formats_and_position() {
        let dir = TempDir::new("date");
        let rule = RenameRule {
            mode: RenameMode::Date,
            date_format: DateFormat::Dashed,
            date_position: DatePosition::Prefix,
            ..Default::default()
        };
        let plan = plan_for(&dir, &["notes.txt", "photo 1.JPG"], &rule);
        assert!(Regex::new(r"^\d{4}-\d{2}-\d{2}_notes\.txt$")
            .unwrap()
            .is_match(&plan.rows[0].new_name));
        assert!(Regex::new(r"^\d{4}-\d{2}-\d{2}_photo 1\.JPG$")
            .unwrap()
            .is_match(&plan.rows[1].new_name));
        assert!(plan.can_apply());

        let rule = RenameRule {
            mode: RenameMode::Date,
            date_format: DateFormat::Compact,
            date_position: DatePosition::Suffix,
            ..Default::default()
        };
        let plan = plan_for(&dir, &["notes.txt"], &rule);
        assert!(Regex::new(r"^notes_\d{8}\.txt$")
            .unwrap()
            .is_match(&plan.rows[0].new_name));
        assert!(plan.rows[0].error.is_none());
    }

    #[test]
    fn detects_existing_and_duplicate_targets() {
        let dir = TempDir::new("conflicts");
        dir.touch("a.txt");
        dir.touch("b.txt");
        dir.touch("x.txt");
        let rule = RenameRule {
            mode: RenameMode::Replace,
            find: "a".into(),
            replace: "x".into(),
            ..Default::default()
        };
        let plan = make_plan(
            &dir.0,
            &file_entries(&["a.txt", "b.txt"]),
            &RenameFilter::default(),
            &rule,
        )
        .unwrap();
        assert!(plan.rows[0].error.as_deref().unwrap().contains("目标名称已存在"));
        assert!(!plan.can_apply());

        let rule = RenameRule {
            mode: RenameMode::Regex,
            find: ".*".into(),
            replace: "same".into(),
            ..Default::default()
        };
        let plan = make_plan(
            &dir.0,
            &file_entries(&["a.txt", "b.txt"]),
            &RenameFilter::default(),
            &rule,
        )
        .unwrap();
        assert_eq!(plan.error_count, 2);
    }

    #[test]
    fn allows_case_only_change() {
        let dir = TempDir::new("case-only");
        let rule = RenameRule {
            mode: RenameMode::Affix,
            prefix: "A".into(),
            ..Default::default()
        };
        let plan = plan_for(&dir, &["apple.txt"], &rule);
        assert_eq!(plan.rows[0].new_name, "Aapple.txt");
        assert!(plan.rows[0].error.is_none());

        let rule = RenameRule {
            mode: RenameMode::Regex,
            find: "^a".into(),
            replace: "A".into(),
            ..Default::default()
        };
        let plan = plan_for(&dir, &["apple2.txt"], &rule);
        assert_eq!(plan.rows[0].new_name, "Apple2.txt");
        assert!(plan.rows[0].error.is_none());
        assert!(plan.can_apply());
    }

    #[test]
    fn rejects_invalid_windows_names() {
        assert!(validate_filename("bad/name.txt").is_some());
        assert!(validate_filename("bad?name.txt").is_some());
        assert!(validate_filename("trailing.").is_some());
        assert!(validate_filename("trailing ").is_some());
        assert!(validate_filename("CON.txt").is_some());
        assert!(validate_filename("lpt9.log").is_some());
        assert!(validate_filename("nul").is_some());
        assert!(validate_filename("console.txt").is_none());
        assert!(validate_filename(".gitignore").is_none());
        assert!(validate_filename("normal.txt").is_none());
    }

    #[test]
    fn applies_real_renames() {
        let dir = TempDir::new("apply");
        dir.touch("b.txt");
        dir.touch("a.txt");
        let rule = RenameRule {
            mode: RenameMode::Numbering,
            template: "{n}".into(),
            digits: 2,
            ..Default::default()
        };
        let mut skipped = 0;
        let files = list_entries(&dir.0, &mut skipped).unwrap();
        let plan = make_plan(&dir.0, &files, &RenameFilter::default(), &rule).unwrap();
        assert!(plan.can_apply());
        let report = apply_plan(&dir.0, &plan);
        assert_eq!(report.renamed, 2);
        assert!(report.failures.is_empty());
        assert_eq!(
            dir.entries(),
            HashSet::from(["01.txt".to_owned(), "02.txt".to_owned()])
        );
    }

    #[test]
    fn applies_case_only_rename() {
        let dir = TempDir::new("apply-case");
        dir.touch("apple.txt");
        let rule = RenameRule {
            mode: RenameMode::Regex,
            find: "^a".into(),
            replace: "A".into(),
            ..Default::default()
        };
        let plan = make_plan(
            &dir.0,
            &file_entries(&["apple.txt"]),
            &RenameFilter::default(),
            &rule,
        )
        .unwrap();
        assert!(plan.can_apply());
        let report = apply_plan(&dir.0, &plan);
        assert_eq!(report.renamed, 1);
        assert!(report.failures.is_empty());
        assert_eq!(dir.entries(), HashSet::from(["Apple.txt".to_owned()]));
    }

    #[test]
    fn extension_filter_selects_files() {
        let dir = TempDir::new("ext-filter");
        for name in ["photo1.JPG", "photo2.png", "notes.txt"] {
            dir.touch(name);
        }
        let rule = RenameRule {
            mode: RenameMode::Replace,
            find: "photo".into(),
            replace: "img".into(),
            ..Default::default()
        };
        let entries = vec![
            Entry {
                name: "notes.txt".into(),
                is_dir: false,
            },
            Entry {
                name: "photo1.JPG".into(),
                is_dir: false,
            },
            Entry {
                name: "photo2.png".into(),
                is_dir: false,
            },
        ];
        let filter = RenameFilter {
            files: true,
            dirs: false,
            extensions: Some(vec!["jpg".into()]),
        };
        let plan = make_plan(&dir.0, &entries, &filter, &rule).unwrap();
        assert_eq!(plan.rows.len(), 1);
        assert_eq!(plan.rows[0].old_name, "photo1.JPG");
        assert_eq!(plan.rows[0].new_name, "img1.JPG");
        assert!(plan.can_apply());
    }

    #[test]
    fn directory_filter_renames_whole_name() {
        let dir = TempDir::new("dir-filter");
        dir.mkdir("archive.2024");
        dir.touch("notes.txt");
        let rule = RenameRule {
            mode: RenameMode::Affix,
            prefix: "old_".into(),
            ..Default::default()
        };
        let mut skipped = 0;
        let entries = list_entries(&dir.0, &mut skipped).unwrap();
        let filter = RenameFilter {
            files: false,
            dirs: true,
            extensions: Some(Vec::new()),
        };
        let plan = make_plan(&dir.0, &entries, &filter, &rule).unwrap();
        assert_eq!(plan.rows.len(), 1);
        assert!(plan.rows[0].is_dir);
        assert_eq!(plan.rows[0].new_name, "old_archive.2024");
        assert!(plan.can_apply());
    }

    #[test]
    fn filtered_out_entries_still_block_conflicts() {
        let dir = TempDir::new("filter-conflict");
        dir.touch("a.jpg");
        dir.touch("x.txt");
        let rule = RenameRule {
            mode: RenameMode::Replace,
            find: "a.jpg".into(),
            replace: "x.txt".into(),
            keep_extension: false,
            ..Default::default()
        };
        let mut skipped = 0;
        let entries = list_entries(&dir.0, &mut skipped).unwrap();
        let filter = RenameFilter {
            files: true,
            dirs: false,
            extensions: Some(vec!["jpg".into()]),
        };
        let plan = make_plan(&dir.0, &entries, &filter, &rule).unwrap();
        assert_eq!(plan.rows.len(), 1);
        assert!(plan.rows[0]
            .error
            .as_deref()
            .unwrap()
            .contains("目标名称已存在"));
        assert!(!plan.can_apply());
    }

    #[test]
    fn numbering_only_counts_selected_entries() {
        let dir = TempDir::new("filter-numbering");
        for name in ["b.jpg", "a.jpg", "c.txt"] {
            dir.touch(name);
        }
        let rule = RenameRule {
            mode: RenameMode::Numbering,
            template: "{n}".into(),
            digits: 2,
            ..Default::default()
        };
        let mut skipped = 0;
        let entries = list_entries(&dir.0, &mut skipped).unwrap();
        let filter = RenameFilter {
            files: true,
            dirs: false,
            extensions: Some(vec!["jpg".into()]),
        };
        let plan = make_plan(&dir.0, &entries, &filter, &rule).unwrap();
        assert_eq!(plan.rows.len(), 2);
        assert_eq!(plan.rows[0].old_name, "a.jpg");
        assert_eq!(plan.rows[0].new_name, "01.jpg");
        assert_eq!(plan.rows[1].new_name, "02.jpg");
    }

    #[test]
    fn incomplete_rule_still_lists_entries() {
        let dir = TempDir::new("preview-listing");
        dir.touch("a.txt");
        dir.mkdir("folder");
        let mut skipped = 0;
        let entries = list_entries(&dir.0, &mut skipped).unwrap();
        let (plan, error) = make_preview_plan(
            &dir.0,
            &entries,
            &RenameFilter::default(),
            &RenameRule::default(),
        );
        assert!(error.as_deref().unwrap().contains("查找"));
        assert_eq!(plan.rows.len(), 1);
        assert_eq!(plan.rows[0].old_name, "a.txt");
        assert_eq!(plan.rows[0].new_name, "a.txt");
        assert!(!plan.rows[0].changed);
        assert!(!plan.can_apply());

        let filter = RenameFilter {
            files: true,
            dirs: true,
            extensions: None,
        };
        let (plan, _) =
            make_preview_plan(&dir.0, &entries, &filter, &RenameRule::default());
        assert_eq!(plan.rows.len(), 2);
        assert!(plan.rows.iter().any(|row| row.is_dir));
    }

    #[test]
    fn applies_directory_rename() {
        let dir = TempDir::new("apply-dir");
        dir.mkdir("old");
        let rule = RenameRule {
            mode: RenameMode::Affix,
            prefix: "new_".into(),
            ..Default::default()
        };
        let mut skipped = 0;
        let entries = list_entries(&dir.0, &mut skipped).unwrap();
        let filter = RenameFilter {
            files: false,
            dirs: true,
            extensions: None,
        };
        let plan = make_plan(&dir.0, &entries, &filter, &rule).unwrap();
        assert!(plan.can_apply());
        let report = apply_plan(&dir.0, &plan);
        assert_eq!(report.renamed, 1);
        assert!(report.failures.is_empty());
        assert!(dir.0.join("new_old").is_dir());
        assert_eq!(dir.entries(), HashSet::from(["new_old".to_owned()]));
    }

    #[test]
    fn natural_sort_handles_numbers_and_case() {
        assert_eq!(natural_cmp("a2", "a10"), Ordering::Less);
        assert_eq!(natural_cmp("A2", "a2"), Ordering::Less);
        assert_eq!(natural_cmp("file09", "file9"), Ordering::Greater);
        assert!(natural_cmp("photo1", "photo01") != Ordering::Equal);
    }
}
