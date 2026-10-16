use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;
use std::sync::Arc;

use pumpkin_nbt::compound::NbtCompound;
use pumpkin_nbt::java_fmt::decimal_format;
use pumpkin_nbt::tag::NbtTag;
use pumpkin_util::text::TextComponent;
use tracing::error;

/// Vanilla `CommandFunction.checkCommandLineLength`'s limit.
const MAX_COMMAND_LENGTH: usize = 2_000_000;
/// The maximum fraction digits of vanilla `MacroFunction.DECIMAL_FORMAT`.
const MACRO_FRACTION_DIGITS: usize = 15;

pub fn load_functions_from_dir<S: std::hash::BuildHasher>(
    namespace: &str,
    function_dir: &Path,
    functions: &mut HashMap<String, Vec<String>, S>,
) {
    if !function_dir.is_dir() {
        return;
    }
    load_functions_recursive(namespace, function_dir, function_dir, functions);
}

fn load_functions_recursive<S: std::hash::BuildHasher>(
    namespace: &str,
    base_dir: &Path,
    current_dir: &Path,
    functions: &mut HashMap<String, Vec<String>, S>,
) {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_functions_recursive(namespace, base_dir, &path, functions);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("mcfunction"))
            && let Ok(rel_path) = path.strip_prefix(base_dir)
        {
            let mut stem_path = rel_path.to_string_lossy().to_string();
            if let Some(s) = stem_path.strip_suffix(".mcfunction") {
                stem_path = s.to_string();
            }
            // Convert Windows backslashes to forward slashes if any
            let stem_path = stem_path.replace('\\', "/");
            let function_id = format!("{namespace}:{stem_path}");
            if let Ok(content) = fs::read_to_string(&path) {
                functions.insert(function_id, content.lines().map(str::to_string).collect());
            }
        }
    }
}

/// Vanilla `ServerFunctionLibrary`'s compile step: every function that `CommandFunction.fromLines`
/// accepts. Plain commands are parsed when they run rather than here.
pub(super) fn compile_functions(
    sources: HashMap<String, Vec<String>>,
) -> HashMap<String, Arc<CommandFunction>> {
    sources
        .into_iter()
        .filter_map(
            |(id, lines)| match CommandFunction::from_lines(&id, &lines) {
                Ok(function) => Some((id, Arc::new(function))),
                Err(reason) => {
                    error!("Failed to load function {id}: {reason}");
                    None
                }
            },
        )
        .collect()
}

/// Vanilla `StringTemplate`: a macro line split around its `$(name)` variables.
#[derive(Debug, PartialEq, Eq)]
pub struct StringTemplate {
    segments: Vec<String>,
    variables: Vec<String>,
}

impl StringTemplate {
    pub fn from_string(input: &str) -> Result<Self, String> {
        let mut segments = Vec::new();
        let mut variables = Vec::new();
        let mut start = 0;
        let mut index = input.find('$');
        while let Some(at) = index {
            if at != input.len() - 1 && input.as_bytes()[at + 1] == b'(' {
                segments.push(input[start..at].to_string());
                let Some(variable_end) = input[at + 1..].find(')').map(|end| end + at + 1) else {
                    return Err("Unterminated macro variable".to_string());
                };
                let variable = &input[at + 2..variable_end];
                if !Self::is_valid_variable_name(variable) {
                    return Err(format!("Invalid macro variable name '{variable}'"));
                }
                variables.push(variable.to_string());
                start = variable_end + 1;
                index = input[start..].find('$').map(|next| next + start);
            } else {
                index = input[at + 1..].find('$').map(|next| next + at + 1);
            }
        }
        if start == 0 {
            return Err("No variables in macro".to_string());
        }
        if start != input.len() {
            segments.push(input[start..].to_string());
        }
        Ok(Self {
            segments,
            variables,
        })
    }

    fn is_valid_variable_name(variable: &str) -> bool {
        variable.chars().all(|c| c.is_alphanumeric() || c == '_')
    }

    pub fn substitute(&self, arguments: &[&str]) -> Result<String, String> {
        let mut command = String::new();
        for (segment, argument) in self.segments.iter().zip(arguments) {
            command.push_str(segment);
            command.push_str(argument);
            check_command_line_length(&command)?;
        }
        if self.segments.len() > self.variables.len()
            && let Some(last) = self.segments.last()
        {
            command.push_str(last);
        }
        check_command_line_length(&command)?;
        Ok(command)
    }
}

/// Vanilla `CommandFunction.checkCommandLineLength`, in UTF-16 units like `CharSequence.length`.
fn check_command_line_length(line: &str) -> Result<(), String> {
    let length = line.encode_utf16().count();
    if length > MAX_COMMAND_LENGTH {
        let truncated: String = line.chars().take(512).collect();
        return Err(format!(
            "Command too long: {length} characters, contents: {truncated}..."
        ));
    }
    Ok(())
}

/// A line of a function: a command, or a macro line with the indices of its variables in the
/// function's parameters.
#[derive(Debug)]
pub enum FunctionEntry {
    Command(String),
    Macro {
        template: StringTemplate,
        parameters: Vec<usize>,
    },
}

/// Why vanilla `CommandFunction.instantiate` failed (`FunctionInstantiationException`).
#[derive(Debug)]
pub struct FunctionInstantiationError(pub TextComponent);

/// Vanilla `CommandFunction`: a `PlainTextFunction` when it has no macro lines, a
/// `MacroFunction` with its parameters otherwise.
#[derive(Debug)]
pub struct CommandFunction {
    id: String,
    entries: Vec<FunctionEntry>,
    /// `None` for a plain function.
    parameters: Option<Vec<String>>,
    plain: Arc<[String]>,
}

impl CommandFunction {
    /// Vanilla `CommandFunction.fromLines`, without compiling the plain commands.
    pub fn from_lines(id: &str, lines: &[String]) -> Result<Self, String> {
        let mut entries = Vec::new();
        let mut parameters: Option<Vec<String>> = None;
        let mut i = 0;
        while i < lines.len() {
            let line_number = i + 1;
            let input_line = lines[i].trim();
            let line = if input_line.ends_with('\\') {
                let mut builder = input_line.to_string();
                loop {
                    i += 1;
                    if i == lines.len() {
                        return Err("Line continuation at end of file".to_string());
                    }
                    builder.pop();
                    builder.push_str(lines[i].trim());
                    check_command_line_length(&builder)?;
                    if !builder.ends_with('\\') {
                        break;
                    }
                }
                builder
            } else {
                input_line.to_string()
            };
            i += 1;
            check_command_line_length(&line)?;

            match line.chars().next() {
                None | Some('#') => {}
                Some('/') => {
                    let rest = &line[1..];
                    if rest.starts_with('/') {
                        return Err(format!(
                            "Unknown or invalid command '{line}' on line {line_number} (if you intended to make a comment, use '#' not '//')"
                        ));
                    }
                    let name: String = rest
                        .chars()
                        .take_while(|&c| c.is_ascii_alphanumeric() || "_-.+".contains(c))
                        .collect();
                    return Err(format!(
                        "Unknown or invalid command '{line}' on line {line_number} (did you mean '{name}'? Do not use a preceding forwards slash.)"
                    ));
                }
                Some('$') => {
                    let command = &line[1..];
                    let template = StringTemplate::from_string(command).map_err(|reason| {
                        format!("Can't parse function line {line_number}: '{command}': {reason}")
                    })?;
                    let names = parameters.get_or_insert_with(Vec::new);
                    let indices = template
                        .variables
                        .iter()
                        .map(|variable| {
                            names
                                .iter()
                                .position(|name| name == variable)
                                .unwrap_or_else(|| {
                                    names.push(variable.clone());
                                    names.len() - 1
                                })
                        })
                        .collect();
                    entries.push(FunctionEntry::Macro {
                        template,
                        parameters: indices,
                    });
                }
                Some(_) => entries.push(FunctionEntry::Command(line)),
            }
        }

        let plain = entries
            .iter()
            .filter_map(|entry| match entry {
                FunctionEntry::Command(command) => Some(command.clone()),
                FunctionEntry::Macro { .. } => None,
            })
            .collect();
        Ok(Self {
            id: id.to_string(),
            entries,
            parameters,
            plain,
        })
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Vanilla `CommandFunction.instantiate`: the commands to run, with the macro lines filled
    /// in from `arguments`. `parse` checks each filled-in macro line the way vanilla compiles it.
    pub fn instantiate(
        &self,
        arguments: Option<&NbtCompound>,
        parse: impl Fn(&str) -> Result<(), String>,
    ) -> Result<Arc<[String]>, FunctionInstantiationError> {
        let Some(parameters) = &self.parameters else {
            return Ok(self.plain.clone());
        };
        let id = TextComponent::text(self.id.clone());
        let Some(arguments) = arguments else {
            return Err(FunctionInstantiationError(translatable(
                "commands.function.error.missing_arguments",
                [id],
            )));
        };
        let mut values = Vec::with_capacity(parameters.len());
        for parameter in parameters {
            let Some(value) = arguments.get(parameter) else {
                return Err(FunctionInstantiationError(translatable(
                    "commands.function.error.missing_argument",
                    [id, TextComponent::text(parameter.clone())],
                )));
            };
            values.push(stringify(value));
        }

        let mut commands = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            match entry {
                FunctionEntry::Command(command) => commands.push(command.clone()),
                FunctionEntry::Macro {
                    template,
                    parameters,
                } => {
                    let arguments: Vec<&str> = parameters
                        .iter()
                        .map(|&index| values[index].as_str())
                        .collect();
                    let command = template.substitute(&arguments).map_err(|reason| {
                        FunctionInstantiationError(TextComponent::text(reason))
                    })?;
                    if let Err(reason) = parse(&command) {
                        return Err(FunctionInstantiationError(translatable(
                            "commands.function.error.parse",
                            [
                                id,
                                TextComponent::text(command),
                                TextComponent::text(reason),
                            ],
                        )));
                    }
                    commands.push(command);
                }
            }
        }
        Ok(commands.into())
    }
}

fn translatable(key: &'static str, args: impl Into<Vec<TextComponent>>) -> TextComponent {
    TextComponent::translate_cross(key, key, args)
}

/// Vanilla `MacroFunction.stringify`.
fn stringify(tag: &NbtTag) -> String {
    match tag {
        NbtTag::Float(value) => decimal_format(f64::from(*value), MACRO_FRACTION_DIGITS),
        NbtTag::Double(value) => decimal_format(*value, MACRO_FRACTION_DIGITS),
        NbtTag::Byte(value) => value.to_string(),
        NbtTag::Short(value) => value.to_string(),
        NbtTag::Long(value) => value.to_string(),
        NbtTag::String(value) => value.to_string(),
        tag => tag.to_string(),
    }
}

/// One entry of a tag file's `values` (vanilla `TagEntry`).
#[derive(Clone, Debug)]
pub struct TagEntry {
    id: String,
    tag: bool,
    required: bool,
}

impl TagEntry {
    fn parse(value: &serde_json::Value) -> Option<Self> {
        let (raw, required) = match value {
            serde_json::Value::String(raw) => (raw.as_str(), true),
            serde_json::Value::Object(object) => (
                object.get("id")?.as_str()?,
                object
                    .get("required")
                    .map_or(Some(true), serde_json::Value::as_bool)?,
            ),
            _ => return None,
        };
        let (id, tag) = raw.strip_prefix('#').map_or((raw, false), |id| (id, true));
        let id = if id.contains(':') {
            id.to_string()
        } else {
            format!("minecraft:{id}")
        };
        Some(Self { id, tag, required })
    }
}

impl std::fmt::Display for TagEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.tag {
            f.write_str("#")?;
        }
        f.write_str(&self.id)?;
        if !self.required {
            f.write_str("?")?;
        }
        Ok(())
    }
}

pub fn load_function_tags_from_dir<S: std::hash::BuildHasher>(
    namespace: &str,
    tags_dir: &Path,
    function_tags: &mut HashMap<String, Vec<TagEntry>, S>,
) {
    let function_tags_dir = tags_dir.join("function");
    let function_tags_dir_plural = tags_dir.join("functions");
    for dir in [&function_tags_dir, &function_tags_dir_plural] {
        if dir.is_dir() {
            load_tags_recursive(namespace, dir, dir, function_tags);
        }
    }
}

/// Vanilla `TagLoader.load` for one directory: a file with `replace` drops what earlier packs
/// added to the tag.
fn load_tags_recursive<S: std::hash::BuildHasher>(
    namespace: &str,
    base_dir: &Path,
    current_dir: &Path,
    tags: &mut HashMap<String, Vec<TagEntry>, S>,
) {
    let Ok(entries) = fs::read_dir(current_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            load_tags_recursive(namespace, base_dir, &path, tags);
        } else if path
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
            && let Ok(rel_path) = path.strip_prefix(base_dir)
        {
            let mut stem_path = rel_path.to_string_lossy().to_string();
            if let Some(s) = stem_path.strip_suffix(".json") {
                stem_path = s.to_string();
            }
            let stem_path = stem_path.replace('\\', "/");
            let tag_id = format!("{namespace}:{stem_path}");
            let parsed = fs::read_to_string(&path)
                .ok()
                .and_then(|content| serde_json::from_str::<serde_json::Value>(&content).ok())
                .and_then(|value| {
                    let entries = value
                        .get("values")?
                        .as_array()?
                        .iter()
                        .map(TagEntry::parse)
                        .collect::<Option<Vec<_>>>()?;
                    let replace = value
                        .get("replace")
                        .map_or(Some(false), serde_json::Value::as_bool)?;
                    Some((entries, replace))
                });
            let Some((entries, replace)) = parsed else {
                error!("Couldn't read tag list {tag_id} from {}", path.display());
                continue;
            };
            let contents = tags.entry(tag_id).or_default();
            if replace {
                contents.clear();
            }
            contents.extend(entries);
        }
    }
}

/// Vanilla `TagLoader.build` for function tags.
///
/// Tags are built after the tags they reference (references that would close a cycle are not
/// followed), each keeps the first occurrence of a function, and a tag missing a required
/// function or tag is not loaded.
pub(super) fn build_function_tags<F>(
    tags: &HashMap<String, Vec<TagEntry>>,
    functions: &HashMap<String, F>,
) -> HashMap<String, Vec<String>> {
    let mut dependencies: HashMap<&str, Vec<&str>> = HashMap::new();
    // Required references first, then optional ones, like DependencySorter.orderByDependencies.
    for required in [true, false] {
        for (id, entries) in tags {
            for entry in entries {
                if entry.tag && entry.required == required {
                    let to = entry.id.as_str();
                    if !is_cyclic(&dependencies, id, to) {
                        let edges = dependencies.entry(id.as_str()).or_default();
                        if !edges.contains(&to) {
                            edges.push(to);
                        }
                    }
                }
            }
        }
    }

    let mut built: HashMap<String, Vec<String>> = HashMap::new();
    let mut visited = HashSet::new();
    for id in tags.keys() {
        visit_tag(id, tags, functions, &dependencies, &mut visited, &mut built);
    }
    built
}

fn is_cyclic(dependencies: &HashMap<&str, Vec<&str>>, from: &str, to: &str) -> bool {
    dependencies.get(to).is_some_and(|deps| {
        deps.contains(&from) || deps.iter().any(|dep| is_cyclic(dependencies, from, dep))
    })
}

fn visit_tag<'a, F>(
    id: &'a str,
    tags: &'a HashMap<String, Vec<TagEntry>>,
    functions: &HashMap<String, F>,
    dependencies: &HashMap<&str, Vec<&'a str>>,
    visited: &mut HashSet<&'a str>,
    built: &mut HashMap<String, Vec<String>>,
) {
    if !visited.insert(id) {
        return;
    }
    for dependency in dependencies.get(id).into_iter().flatten() {
        visit_tag(dependency, tags, functions, dependencies, visited, built);
    }
    let Some(entries) = tags.get(id) else {
        return;
    };
    let mut values: Vec<String> = Vec::new();
    let mut missing = Vec::new();
    for entry in entries {
        let found = if entry.tag {
            built.get(&entry.id).cloned()
        } else {
            functions
                .contains_key(&entry.id)
                .then(|| vec![entry.id.clone()])
        };
        match found {
            Some(ids) => {
                for function in ids {
                    if !values.contains(&function) {
                        values.push(function);
                    }
                }
            }
            None if entry.required => missing.push(entry.to_string()),
            None => {}
        }
    }
    if missing.is_empty() {
        built.insert(id.to_string(), values);
    } else {
        error!(
            "Couldn't load tag {id} as it is missing following references: {}",
            missing.join(", ")
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(content: &str) -> Vec<String> {
        content.lines().map(str::to_string).collect()
    }

    #[test]
    fn plain_functions_skip_comments_and_join_continued_lines() {
        let function = CommandFunction::from_lines(
            "t:f",
            &lines("# comment\n  say Hello world  \n\nsay a \\\n  b\\\nc\n"),
        )
        .unwrap();
        let commands = function.instantiate(None, |_| Ok(())).unwrap();
        assert_eq!(&*commands, ["say Hello world", "say a bc"]);
    }

    #[test]
    fn leading_slashes_fail_the_function() {
        assert!(CommandFunction::from_lines("t:f", &lines("/say hi")).is_err());
        assert!(CommandFunction::from_lines("t:f", &lines("//say hi")).is_err());
        assert!(CommandFunction::from_lines("t:f", &lines("say hi \\")).is_err());
        assert!(CommandFunction::from_lines("t:f", &lines("$say no variables")).is_err());
        assert!(CommandFunction::from_lines("t:f", &lines("$say $(a-b)")).is_err());
        assert!(CommandFunction::from_lines("t:f", &lines("$say $(open")).is_err());
    }

    #[test]
    fn macros_substitute_their_arguments() {
        let function = CommandFunction::from_lines(
            "t:f",
            &lines("say first\n$say $(a) and $(b)$(a)!\n$tp @s $(x) ~ ~"),
        )
        .unwrap();
        let mut arguments = NbtCompound::new();
        arguments.put_string("a", "A".to_string());
        arguments.put_byte("b", 3);
        arguments.put_double("x", 0.1 + 0.2);
        let commands = function.instantiate(Some(&arguments), |_| Ok(())).unwrap();
        assert_eq!(&*commands, ["say first", "say A and 3A!", "tp @s .3 ~ ~"]);
        assert!(function.instantiate(None, |_| Ok(())).is_err());
        assert!(
            function
                .instantiate(Some(&NbtCompound::new()), |_| Ok(()))
                .is_err()
        );
    }
}
