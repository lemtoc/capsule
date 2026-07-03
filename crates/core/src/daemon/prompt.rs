use crate::{
    config::{Config, ModuleSlot, PromptLayout, PromptSide},
    module::{
        CmdDurationModule, CustomModuleInfo, DirectoryModule, Module, RenderContext, StatusModule,
        TimeModule,
    },
    render::{
        PromptLines, append_right_aligned, compose_segment_line, compose_segments_with_widths,
        display_width,
        segment::Segment,
        style::{Color, ColorMap, Style},
        truncate,
    },
};

/// ASCII Record Separator — delimits key from value in `char_meta` entries.
const RS: char = '\x1e';
/// ASCII Unit Separator — delimits entries in `char_meta`.
const US: char = '\x1f';
const RIGHT_PROMPT_INDENT: usize = 1;

#[derive(Debug, Clone)]
pub(super) struct FastOutputs {
    directory: Option<String>,
    cmd_duration: Option<String>,
    time: Option<String>,
    status: Option<String>,
    character: Option<String>,
    last_exit_code: i32,
    read_only: bool,
    custom_modules: Vec<CustomModuleInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SlowOutput {
    pub(super) git: Option<String>,
    pub(super) custom_modules: Vec<CustomModuleInfo>,
}

/// Compute built-in fast modules and combine with pre-detected custom modules.
///
/// Custom module detection is done by the caller (potentially in parallel).
pub(super) fn run_fast_modules(
    ctx: &RenderContext<'_>,
    config: &Config,
    read_only: bool,
    custom_modules: Vec<CustomModuleInfo>,
) -> FastOutputs {
    let time = if config.time.disabled {
        None
    } else {
        TimeModule::with_show_seconds(config.time.show_seconds())
            .render(ctx)
            .map(|output| output.content)
    };
    FastOutputs {
        directory: if config.directory.disabled {
            None
        } else {
            DirectoryModule::new()
                .render(ctx)
                .map(|output| output.content)
        },
        cmd_duration: if config.cmd_duration.disabled {
            None
        } else {
            CmdDurationModule::with_threshold(config.cmd_duration.threshold_ms)
                .render(ctx)
                .map(|output| output.content)
        },
        time,
        status: StatusModule::new().render(ctx).map(|output| output.content),
        character: if config.character.disabled {
            None
        } else {
            Some(config.character.glyph.clone())
        },
        last_exit_code: ctx.last_exit_code,
        read_only,
        custom_modules,
    }
}

/// Prompt layout (Starship-compatible):
/// - Info line (left1):  `[directory] on [git] via [toolchain] [cmd_duration]`
/// - Input line (left2): `at [time] [character]`
#[cfg(test)]
fn compose_prompt(
    fast: &FastOutputs,
    slow: Option<&SlowOutput>,
    cols: usize,
    config: &Config,
) -> PromptLines {
    compose_prompt_with_prefix(fast, slow, cols, 0, config)
}

pub(super) fn compose_prompt_with_prefix(
    fast: &FastOutputs,
    slow: Option<&SlowOutput>,
    cols: usize,
    line1_prefix_cols: usize,
    config: &Config,
) -> PromptLines {
    match config.layout {
        PromptLayout::TwoLine => {
            compose_two_line_prompt(fast, slow, cols, line1_prefix_cols, config)
        }
        PromptLayout::Fish => compose_fish_prompt(fast, slow, cols, config),
    }
}

fn compose_two_line_prompt(
    fast: &FastOutputs,
    slow: Option<&SlowOutput>,
    cols: usize,
    line1_prefix_cols: usize,
    config: &Config,
) -> PromptLines {
    let connector_style = config.connectors.prompt_style();

    let mut line1 = Vec::with_capacity(4);

    if let Some(dir) = &fast.directory {
        line1.push(
            config
                .directory
                .to_segment(dir, fast.read_only, config.color_map),
        );
    }

    if let Some(git) = slow.and_then(|output| output.git.as_deref()) {
        line1.push(config.git.to_segment(git, connector_style));
    }

    let slow_custom_modules = slow.map(|output| output.custom_modules.as_slice());

    append_custom_modules(
        &mut line1,
        &fast.custom_modules,
        slow_custom_modules,
        ModuleSlot::Line1,
        connector_style,
    );

    if let Some(duration) = &fast.cmd_duration {
        line1.push(config.cmd_duration.to_segment(duration, connector_style));
    }

    let mut line2 = Vec::with_capacity(2);
    let mut right1 = Vec::with_capacity(1);
    let mut right2 = Vec::with_capacity(1);

    append_custom_modules(
        &mut line2,
        &fast.custom_modules,
        slow_custom_modules,
        ModuleSlot::Line2,
        connector_style,
    );

    if let Some(time) = &fast.time {
        let segment = config.time.to_segment(time, connector_style);
        match (config.time.slot, config.time.side) {
            (ModuleSlot::Line1, PromptSide::Left) => line1.push(segment),
            (ModuleSlot::Line1, PromptSide::Right) => right1.push(segment),
            (ModuleSlot::Line2, PromptSide::Left) => line2.push(segment),
            (ModuleSlot::Line2, PromptSide::Right) => right2.push(segment),
        }
    }

    if let Some(status) = &fast.status {
        line2.push(status_segment(status));
    }

    let viins_seg = character_segment(fast, config);
    if let Some(ref seg) = viins_seg {
        line2.push(seg.clone());
    }

    let line1_cols = cols.saturating_sub(line1_prefix_cols);
    let mut right1_rendered = compose_segment_line(&right1, line1_cols, config.color_map);
    let line1_render_cols = if right1_rendered.is_empty() {
        line1_cols
    } else {
        line1_cols.saturating_sub(RIGHT_PROMPT_INDENT)
    };

    let mut result =
        compose_segments_with_widths(&line1, &line2, line1_render_cols, cols, config.color_map);
    result.right1 = std::mem::take(&mut right1_rendered);
    result.right2 = compose_segment_line(&right2, cols, config.color_map);
    append_right_aligned(
        &mut result.left1,
        &result.right1,
        line1_cols,
        RIGHT_PROMPT_INDENT,
    );
    result.right1.clear();

    if let Some(viins) = &viins_seg {
        apply_prompt_meta(&mut result, Some(viins), config, fast.last_exit_code);
    } else {
        apply_prompt_meta(&mut result, None, config, fast.last_exit_code);
    }

    result
}

fn compose_fish_prompt(
    fast: &FastOutputs,
    slow: Option<&SlowOutput>,
    cols: usize,
    config: &Config,
) -> PromptLines {
    let connector_style = config.connectors.prompt_style();
    let mut core = Vec::with_capacity(2);

    if let Some(dir) = &fast.directory {
        core.push(
            config
                .directory
                .to_segment(dir, fast.read_only, config.color_map),
        );
    }

    if let Some(git) = slow.and_then(|output| output.git.as_deref()) {
        core.push(Segment {
            content: format!("({git})"),
            connector: None,
            icon: None,
            content_style: None,
        });
    }

    let slow_custom_modules = slow.map(|output| output.custom_modules.as_slice());
    let mut optional = Vec::new();
    append_custom_modules(
        &mut optional,
        &fast.custom_modules,
        slow_custom_modules,
        ModuleSlot::Line1,
        connector_style,
    );

    if let Some(duration) = &fast.cmd_duration {
        optional.push(config.cmd_duration.to_segment(duration, connector_style));
    }

    let mut tail = Vec::with_capacity(2);
    if let Some(status) = &fast.status {
        tail.push(status_segment(status));
    }

    let viins_seg = character_segment(fast, config);
    if let Some(ref seg) = viins_seg {
        tail.push(seg.clone());
    }

    let mut result = PromptLines {
        left1: compose_fish_line(&core, &optional, &tail, cols, config.color_map),
        left2: String::new(),
        right1: String::new(),
        right2: String::new(),
        char_meta: String::new(),
    };

    if let Some(viins) = &viins_seg {
        apply_prompt_meta(&mut result, Some(viins), config, fast.last_exit_code);
    }

    result
}

fn status_segment(status: &str) -> Segment {
    Segment {
        content: format!("[{status}]"),
        connector: None,
        icon: None,
        content_style: Some(Style::new().fg(Color::Red).bold()),
    }
}

fn compose_fish_line(
    core: &[Segment],
    optional: &[Segment],
    tail: &[Segment],
    cols: usize,
    color_map: ColorMap,
) -> String {
    if cols == 0 {
        return String::new();
    }

    for optional_len in (0..=optional.len()).rev() {
        let mut segments = Vec::with_capacity(core.len() + optional_len + tail.len());
        segments.extend_from_slice(core);
        segments.extend_from_slice(&optional[..optional_len]);
        segments.extend_from_slice(tail);

        let rendered = render_prompt_segments(&segments, color_map);
        let joined = join_prompt_parts(&rendered);
        if display_width(&joined) <= cols {
            return joined;
        }
    }

    compose_with_preserved_tail(core, tail, cols, color_map)
}

fn compose_with_preserved_tail(
    core: &[Segment],
    tail: &[Segment],
    cols: usize,
    color_map: ColorMap,
) -> String {
    let rendered_core = render_prompt_segments(core, color_map);
    let rendered_tail = render_prompt_segments(tail, color_map);
    let core_joined = join_prompt_parts(&rendered_core);
    let tail_joined = join_prompt_parts(&rendered_tail);

    if tail_joined.is_empty() {
        return truncate(&core_joined, cols);
    }

    let tail_width = display_width(&tail_joined);
    if tail_width >= cols {
        return truncate(&tail_joined, cols);
    }

    let separator_width = usize::from(!core_joined.is_empty());
    let available_core_width = cols.saturating_sub(tail_width + separator_width);
    let core_fit = truncate(&core_joined, available_core_width);

    if core_fit.is_empty() {
        tail_joined
    } else {
        format!("{core_fit} {tail_joined}")
    }
}

fn render_prompt_segments(segments: &[Segment], color_map: ColorMap) -> Vec<String> {
    segments
        .iter()
        .map(|segment| segment.render(color_map))
        .collect()
}

fn join_prompt_parts(parts: &[String]) -> String {
    let mut out = String::new();
    for part in parts.iter().filter(|part| !part.is_empty()) {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(part);
    }
    out
}

fn character_segment(fast: &FastOutputs, config: &Config) -> Option<Segment> {
    fast.character
        .as_deref()
        .map(|glyph| config.character.to_segment(glyph, fast.last_exit_code))
}

fn apply_prompt_meta(
    result: &mut PromptLines,
    viins: Option<&Segment>,
    config: &Config,
    exit_code: i32,
) {
    let mut entries = Vec::new();

    if !result.right1.is_empty() {
        entries.push(format!("right1{RS}{}", result.right1));
    }
    if !result.right2.is_empty() {
        entries.push(format!("right2{RS}{}", result.right2));
    }

    if let Some(viins) = viins {
        let vicmd_seg = config
            .character
            .mode_segment(&config.character.vicmd, exit_code);
        let viins_styled = viins.render(config.color_map);
        let vicmd_styled = vicmd_seg.render(config.color_map);
        entries.push(format!("viins{RS}{viins_styled}"));
        entries.push(format!("vicmd{RS}{vicmd_styled}"));
    }

    let separator = US.to_string();
    result.char_meta = entries.join(&separator);
}

fn append_custom_modules(
    line: &mut Vec<Segment>,
    fast_modules: &[CustomModuleInfo],
    slow_modules: Option<&[CustomModuleInfo]>,
    slot: ModuleSlot,
    connector_style: Style,
) {
    for module in fast_modules
        .iter()
        .chain(slow_modules.unwrap_or(&[]).iter())
        .filter(|module| module.slot == slot)
    {
        line.push(module.to_segment(connector_style));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        config::Config,
        module::preset_module_defs,
        render::style::{Color, Style},
        test_utils::contains_style_sequence,
    };

    fn default_config() -> Config {
        Config::default()
    }

    fn make_fast_outputs() -> FastOutputs {
        FastOutputs {
            directory: Some("/tmp".to_owned()),
            cmd_duration: None,
            time: None,
            status: None,
            character: Some("\u{276f}".to_owned()),
            last_exit_code: 0,
            read_only: false,
            custom_modules: vec![],
        }
    }

    fn make_slow_output() -> SlowOutput {
        SlowOutput {
            git: None,
            custom_modules: vec![],
        }
    }

    fn make_toolchain_module(name: &str, version: &str) -> CustomModuleInfo {
        let presets = preset_module_defs();
        let preset = presets.iter().find(|def| def.name == name);
        let style = preset.map_or(Style::new().fg(Color::BrightBlack), |def| {
            def.style
                .resolve(Style::new().fg(Color::BrightBlack).bold())
        });
        CustomModuleInfo {
            name: name.to_owned(),
            value: version.to_owned(),
            icon: preset.and_then(|def| def.icon.clone()),
            style,
            connector: Some("via".to_owned()),
            slot: ModuleSlot::default(),
        }
    }

    fn make_line2_module(name: &str, version: &str) -> CustomModuleInfo {
        let mut module = make_toolchain_module(name, version);
        module.slot = ModuleSlot::Line2;
        module
    }

    fn contains_yellow_ansi(line: &str) -> bool {
        line.contains("\x1b[33m")
            || contains_style_sequence(line, &[1, 33])
            || contains_style_sequence(line, &[33, 1])
    }

    fn fish_config() -> Config {
        let mut config = default_config();
        config.layout = PromptLayout::Fish;
        config.character.glyph = ">".to_owned();
        config
    }

    #[test]
    fn test_fast_only() {
        let fast = FastOutputs {
            time: Some("14:30:45".to_owned()),
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(lines.left1.contains("/tmp"), "left1: {}", lines.left1);
        assert!(
            lines.left2.contains("at"),
            "left2 should have 'at': {}",
            lines.left2
        );
        assert!(
            lines.left2.contains("14:30:45"),
            "left2 should have time: {}",
            lines.left2
        );
        assert!(
            lines.left2.contains('\u{276f}'),
            "left2 should have character: {}",
            lines.left2
        );
    }

    #[test]
    fn test_time_can_render_on_right2() {
        let fast = FastOutputs {
            time: Some("14:30".to_owned()),
            ..make_fast_outputs()
        };
        let mut config = default_config();
        config.time.slot = ModuleSlot::Line2;
        config.time.side = PromptSide::Right;
        config.time.connector = String::new();

        let lines = compose_prompt(&fast, None, 80, &config);

        assert!(
            !lines.left2.contains("14:30"),
            "left2 should not contain right-aligned time: {}",
            lines.left2
        );
        assert!(
            lines.right2.contains("14:30"),
            "right2 should contain time: {}",
            lines.right2
        );
        assert!(
            lines.char_meta.contains("right2\x1e"),
            "metadata should contain right2: {}",
            lines.char_meta
        );
    }

    #[test]
    fn test_time_can_render_on_right1() {
        let fast = FastOutputs {
            time: Some("14:30".to_owned()),
            ..make_fast_outputs()
        };
        let mut config = default_config();
        config.time.slot = ModuleSlot::Line1;
        config.time.side = PromptSide::Right;
        config.time.connector = String::new();

        let lines = compose_prompt(&fast, None, 30, &config);

        assert!(
            lines.left1.contains("14:30"),
            "left1 should contain right-aligned time: {}",
            lines.left1
        );
        assert_eq!(
            display_width(&lines.left1),
            29,
            "left1 should leave right prompt indent: {}",
            lines.left1
        );
        assert!(
            !lines.char_meta.contains("right1\x1e"),
            "line1 right prompt should be materialized into left1: {}",
            lines.char_meta
        );
    }

    #[test]
    fn test_time_on_right1_reserves_prefix_width() {
        let fast = FastOutputs {
            time: Some("14:30".to_owned()),
            ..make_fast_outputs()
        };
        let mut config = default_config();
        config.time.slot = ModuleSlot::Line1;
        config.time.side = PromptSide::Right;
        config.time.connector = String::new();

        let lines = compose_prompt_with_prefix(&fast, None, 30, 10, &config);

        assert!(
            lines.left1.contains("14:30"),
            "left1 should contain right-aligned time: {}",
            lines.left1
        );
        assert_eq!(
            display_width(&lines.left1),
            19,
            "left1 should fit after prefix and indent: {}",
            lines.left1
        );
    }

    #[test]
    fn test_with_slow() {
        let fast = make_fast_outputs();
        let slow = SlowOutput {
            git: Some("main".to_owned()),
            ..make_slow_output()
        };
        let lines = compose_prompt(&fast, Some(&slow), 80, &default_config());
        assert!(lines.left1.contains("/tmp"), "left1: {}", lines.left1);
        assert!(
            lines.left1.contains("on"),
            "left1 should contain 'on' connector: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains("main"),
            "left1 should contain branch: {}",
            lines.left1
        );
    }

    #[test]
    fn test_none_git() {
        let fast = make_fast_outputs();
        let slow = make_slow_output();
        let without_slow = compose_prompt(&fast, None, 80, &default_config());
        let with_none_git = compose_prompt(&fast, Some(&slow), 80, &default_config());
        assert_eq!(without_slow, with_none_git);
    }

    #[test]
    fn test_directory_style() {
        let fast = make_fast_outputs();
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            contains_style_sequence(&lines.left1, &[1, 36]),
            "directory should be bold cyan: {}",
            lines.left1
        );
    }

    #[test]
    fn test_character_success_style() {
        let fast = make_fast_outputs();
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.left2.contains("\x1b[32m"),
            "character should be green on success: {}",
            lines.left2
        );
    }

    #[test]
    fn test_character_error_style() {
        let fast = FastOutputs {
            last_exit_code: 1,
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.left2.contains("\x1b[31m"),
            "character should be red on error: {}",
            lines.left2
        );
    }

    #[test]
    fn test_status_on_line2() {
        let fast = FastOutputs {
            status: Some("1".to_owned()),
            last_exit_code: 1,
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.left2.contains("[1]"),
            "line2 should contain non-zero status: {}",
            lines.left2
        );
        assert!(
            lines.left2.contains("\x1b[31m"),
            "status should be red: {}",
            lines.left2
        );
    }

    #[test]
    fn test_line2_module_on_left2_only() {
        let mut fast = make_fast_outputs();
        fast.custom_modules = vec![make_line2_module("node", "v20.0")];
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.left2.contains("v20.0"),
            "line2 module should appear on left2: {}",
            lines.left2
        );
        assert!(
            !lines.left1.contains("v20.0"),
            "line2 module should not appear on left1: {}",
            lines.left1
        );
    }

    #[test]
    fn test_line1_module_not_on_left2() {
        let fast = make_fast_outputs();
        let slow = SlowOutput {
            custom_modules: vec![make_toolchain_module("rust", "v1.82.0")],
            ..make_slow_output()
        };
        let lines = compose_prompt(&fast, Some(&slow), 80, &default_config());
        assert!(
            lines.left1.contains("v1.82.0"),
            "line1 module should appear on left1: {}",
            lines.left1
        );
        assert!(
            !lines.left2.contains("v1.82.0"),
            "line1 module should not appear on left2: {}",
            lines.left2
        );
    }

    #[test]
    fn test_mixed_slots_both_lines() {
        let mut fast = make_fast_outputs();
        fast.custom_modules = vec![
            make_toolchain_module("rust", "v1.82.0"),
            make_line2_module("node", "v20.0"),
        ];
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.left1.contains("v1.82.0"),
            "line1 module should appear on left1: {}",
            lines.left1
        );
        assert!(
            !lines.left1.contains("v20.0"),
            "line2 module should not appear on left1: {}",
            lines.left1
        );
        assert!(
            lines.left2.contains("v20.0"),
            "line2 module should appear on left2: {}",
            lines.left2
        );
        assert!(
            !lines.left2.contains("v1.82.0"),
            "line1 module should not appear on left2: {}",
            lines.left2
        );
    }

    #[test]
    fn test_toolchain_style() {
        let fast = make_fast_outputs();
        let slow = SlowOutput {
            custom_modules: vec![make_toolchain_module("rust", "v1.82.0")],
            ..make_slow_output()
        };
        let lines = compose_prompt(&fast, Some(&slow), 80, &default_config());
        assert!(
            lines.left1.contains("via"),
            "left1 should contain 'via' connector: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains("v1.82.0"),
            "left1 should contain version: {}",
            lines.left1
        );
        assert!(
            !lines.left1.contains("rust"),
            "left1 should not contain toolchain name: {}",
            lines.left1
        );
        assert!(
            contains_style_sequence(&lines.left1, &[1, 31]),
            "rust toolchain should use bold red: {}",
            lines.left1
        );
    }

    #[test]
    fn test_toolchain_omitted_without_slow() {
        let fast = make_fast_outputs();
        let without_slow = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            !without_slow.left1.contains("via"),
            "toolchain should not appear without slow output: {}",
            without_slow.left1
        );

        let empty_slow = SlowOutput {
            custom_modules: vec![],
            ..make_slow_output()
        };
        let with_empty_slow = compose_prompt(&fast, Some(&empty_slow), 80, &default_config());
        assert!(
            !with_empty_slow.left1.contains("via"),
            "toolchain should not appear with empty slow output: {}",
            with_empty_slow.left1
        );
    }

    #[test]
    fn test_time_on_line2() {
        let fast = FastOutputs {
            time: Some("14:30:45".to_owned()),
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            !lines.left1.contains("14:30:45"),
            "time should not be on line 1: {}",
            lines.left1
        );
        assert!(
            lines.left2.contains("14:30:45"),
            "time should be on line 2: {}",
            lines.left2
        );
        assert!(
            contains_yellow_ansi(&lines.left2),
            "time should use yellow styling: {}",
            lines.left2
        );
    }

    #[test]
    fn test_connector_styles() {
        let fast = FastOutputs {
            time: Some("14:30:45".to_owned()),
            ..make_fast_outputs()
        };
        let slow = SlowOutput {
            git: Some("main".to_owned()),
            custom_modules: vec![make_toolchain_module("rust", "v1.82.0")],
        };
        let lines = compose_prompt(&fast, Some(&slow), 80, &default_config());
        assert!(
            lines.left1.contains("on"),
            "git connector should be present: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains("via"),
            "toolchain connector should be present: {}",
            lines.left1
        );
        assert!(
            lines.left2.contains("at"),
            "time connector should be present: {}",
            lines.left2
        );
        assert!(
            !lines.left1.contains("\x1b[90mon\x1b[0m")
                && !lines.left1.contains("\x1b[90mvia\x1b[0m"),
            "connectors should not use bright black: {}",
            lines.left1
        );
        assert!(
            !lines.left2.contains("\x1b[90mat\x1b[0m"),
            "time connector should not use bright black: {}",
            lines.left2
        );
        assert!(
            contains_yellow_ansi(&lines.left2),
            "time content should use yellow styling: {}",
            lines.left2
        );
    }

    #[test]
    fn test_branch_icon() {
        let fast = make_fast_outputs();
        let slow = SlowOutput {
            git: Some("main".to_owned()),
            ..make_slow_output()
        };
        let lines = compose_prompt(&fast, Some(&slow), 80, &default_config());
        assert!(
            lines.left1.contains('\u{f418}'),
            "branch icon should be \\u{{f418}}: {}",
            lines.left1
        );
    }

    #[test]
    fn test_cmd_duration_connector() {
        let fast = FastOutputs {
            cmd_duration: Some("3s".to_owned()),
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.left1.contains("took"),
            "cmd_duration should have 'took' connector: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains("3s"),
            "cmd_duration should contain duration: {}",
            lines.left1
        );
    }

    #[test]
    fn test_readonly_lock_style() {
        let fast = FastOutputs {
            read_only: true,
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.left1.contains('\u{f023}'),
            "readonly dir should show lock icon: {}",
            lines.left1
        );
        let lock_pos = lines.left1.find('\u{f023}');
        assert!(lock_pos.is_some(), "lock icon should be present");
        let before_lock = &lines.left1[..lock_pos.unwrap_or(0)];
        assert!(
            before_lock.contains("\x1b[31m"),
            "lock icon should be styled red: {}",
            lines.left1
        );
    }

    #[test]
    fn test_writable_no_lock_icon() {
        let lines = compose_prompt(&make_fast_outputs(), None, 80, &default_config());
        assert!(
            !lines.left1.contains('\u{f023}'),
            "writable dir should not show lock icon: {}",
            lines.left1
        );
    }

    #[test]
    fn test_custom_character_glyph() {
        let fast = FastOutputs {
            character: Some("$".to_owned()),
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.left2.contains('$'),
            "left2 should contain custom glyph '$': {}",
            lines.left2
        );
    }

    #[test]
    fn test_custom_character_colors() {
        let fast = make_fast_outputs();
        let mut config = default_config();
        config.character.success_style.fg = Some(Color::Magenta);
        let lines = compose_prompt(&fast, None, 80, &config);
        assert!(
            lines.left2.contains("\x1b[35m"),
            "character should use magenta on success: {}",
            lines.left2
        );
    }

    #[test]
    fn test_custom_directory_color() {
        let fast = make_fast_outputs();
        let mut config = default_config();
        config.directory.style.fg = Some(Color::Green);
        let lines = compose_prompt(&fast, None, 80, &config);
        assert!(
            contains_style_sequence(&lines.left1, &[1, 32]),
            "directory should use bold green: {}",
            lines.left1
        );
    }

    #[test]
    fn test_custom_connectors() {
        let fast = FastOutputs {
            time: Some("14:30:45".to_owned()),
            cmd_duration: Some("3s".to_owned()),
            ..make_fast_outputs()
        };
        let slow = SlowOutput {
            git: Some("main".to_owned()),
            ..make_slow_output()
        };
        let mut config = default_config();
        config.git.connector = "branch".to_owned();
        config.time.connector = "time".to_owned();
        config.cmd_duration.connector = "duration".to_owned();
        let lines = compose_prompt(&fast, Some(&slow), 80, &config);
        assert!(
            lines.left1.contains("branch"),
            "git connector should be 'branch': {}",
            lines.left1
        );
        assert!(
            lines.left1.contains("duration"),
            "cmd_duration connector should be 'duration': {}",
            lines.left1
        );
        assert!(
            lines.left2.contains("time"),
            "time connector should be 'time': {}",
            lines.left2
        );
    }

    #[test]
    fn test_time_disabled() {
        let fast = FastOutputs {
            time: None,
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            !lines.left2.contains("at"),
            "time connector should not appear when time is None: {}",
            lines.left2
        );
    }

    #[test]
    fn test_empty_time_connector_omits_connector() {
        let fast = FastOutputs {
            time: Some("17:44".to_owned()),
            ..make_fast_outputs()
        };
        let mut config = default_config();
        config.time.connector = String::new();
        let lines = compose_prompt(&fast, None, 80, &config);
        assert!(
            lines.left2.contains("17:44"),
            "line2 should contain time: {}",
            lines.left2
        );
        assert!(
            !lines.left2.contains("at"),
            "time connector should be omitted: {}",
            lines.left2
        );
    }

    #[test]
    fn test_custom_git_icon() {
        let fast = make_fast_outputs();
        let slow = SlowOutput {
            git: Some("main".to_owned()),
            ..make_slow_output()
        };
        let mut config = default_config();
        config.git.icon = "\u{e0a0}".to_owned();
        let lines = compose_prompt(&fast, Some(&slow), 80, &config);
        assert!(
            lines.left1.contains('\u{e0a0}'),
            "git icon should be custom icon: {}",
            lines.left1
        );
        assert!(
            !lines.left1.contains('\u{f418}'),
            "default git icon should not appear: {}",
            lines.left1
        );
    }

    #[test]
    fn test_empty_git_connector_keeps_icon_without_connector() {
        let fast = make_fast_outputs();
        let slow = SlowOutput {
            git: Some("main".to_owned()),
            ..make_slow_output()
        };
        let mut config = default_config();
        config.git.connector = String::new();
        let lines = compose_prompt(&fast, Some(&slow), 80, &config);
        assert!(
            !lines.left1.contains("on"),
            "git connector should be omitted: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains('\u{f418}'),
            "git icon should remain: {}",
            lines.left1
        );
    }

    #[test]
    fn test_custom_cmd_duration_color() {
        let fast = FastOutputs {
            cmd_duration: Some("3s".to_owned()),
            ..make_fast_outputs()
        };
        let mut config = default_config();
        config.cmd_duration.style.fg = Some(Color::Red);
        let lines = compose_prompt(&fast, None, 80, &config);
        assert!(
            contains_style_sequence(&lines.left1, &[1, 31])
                || contains_style_sequence(&lines.left1, &[31, 1]),
            "cmd_duration should use bold red: {}",
            lines.left1
        );
    }

    #[test]
    fn test_structured_styles_and_color_map() {
        let fast = FastOutputs {
            time: Some("14:30:45".to_owned()),
            cmd_duration: Some("3s".to_owned()),
            read_only: true,
            ..make_fast_outputs()
        };
        let slow = SlowOutput {
            git: Some("main [!+]".to_owned()),
            custom_modules: vec![make_toolchain_module("rust", "v1.82.0")],
        };
        let mut config = default_config();
        config.directory.style.fg = Some(Color::Blue);
        config.directory.style.bold = Some(false);
        config.directory.read_only_style.fg = Some(Color::Yellow);
        config.directory.read_only_style.bold = Some(true);
        config.git.style.fg = Some(Color::Cyan);
        config.git.style.bold = Some(false);
        config.git.indicator_style.fg = Some(Color::Yellow);
        config.git.indicator_style.bold = Some(false);
        config.time.style.fg = Some(Color::Blue);
        config.time.style.dimmed = Some(true);
        config.cmd_duration.style.fg = Some(Color::Yellow);
        config.cmd_duration.style.bold = Some(true);
        config.character.success_style.fg = Some(Color::Magenta);
        config.character.success_style.bold = Some(true);
        config.connectors.style.fg = Some(Color::BrightBlack);
        config.connectors.style.dimmed = Some(true);
        config.color_map.blue = 94;
        config.color_map.yellow = 93;
        config.color_map.magenta = 95;
        config.color_map.cyan = 96;
        config.color_map.bright_black = 37;

        let lines = compose_prompt(&fast, Some(&slow), 120, &config);

        assert!(
            lines.left1.contains("\x1b[94m"),
            "directory should use remapped blue: {}",
            lines.left1
        );
        assert!(
            contains_style_sequence(&lines.left1, &[1, 93])
                || contains_style_sequence(&lines.left1, &[93, 1]),
            "read-only lock should use bold remapped yellow: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains("\x1b[96m"),
            "git branch/icon should use remapped cyan: {}",
            lines.left1
        );
        assert!(
            contains_style_sequence(&lines.left1, &[37, 2])
                || contains_style_sequence(&lines.left1, &[2, 37]),
            "connectors should use configured dimmed bright_black mapping: {}",
            lines.left1
        );
        assert!(
            contains_style_sequence(&lines.left2, &[2, 94])
                || contains_style_sequence(&lines.left2, &[94, 2]),
            "time should use dimmed remapped blue: {}",
            lines.left2
        );
        assert!(
            contains_style_sequence(&lines.left2, &[1, 95])
                || contains_style_sequence(&lines.left2, &[95, 1]),
            "character should use bold remapped magenta: {}",
            lines.left2
        );
    }

    #[test]
    fn test_char_meta_empty_when_disabled() {
        let fast = FastOutputs {
            character: None,
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            lines.char_meta.is_empty(),
            "char_meta should be empty when character is disabled"
        );
    }

    #[test]
    fn test_char_meta_with_default_config() {
        let fast = make_fast_outputs();
        let lines = compose_prompt(&fast, None, 80, &default_config());
        assert!(
            !lines.char_meta.is_empty(),
            "char_meta should be populated by default"
        );
        assert!(
            lines.char_meta.contains("viins\x1e"),
            "char_meta should contain viins entry"
        );
        assert!(
            lines.char_meta.contains("\x1fvicmd\x1e"),
            "char_meta should contain vicmd entry"
        );
    }

    #[test]
    fn test_char_meta_custom_vicmd_style() {
        let fast = make_fast_outputs();
        let mut config = default_config();
        config.character.vicmd = crate::config::CharacterModeConfig {
            glyph: "❮".to_owned(),
            style: Some(crate::config::StyleConfig::fg(Color::Green)),
        };
        let lines = compose_prompt(&fast, None, 80, &config);
        assert!(
            !lines.char_meta.is_empty(),
            "char_meta should be populated when vicmd has custom style"
        );
    }

    #[test]
    fn test_char_meta_viins_matches_left2() {
        let fast = make_fast_outputs();
        let lines = compose_prompt(&fast, None, 80, &default_config());
        // Extract viins styled string from char_meta
        let viins_entry = lines
            .char_meta
            .split('\x1f')
            .find(|e| e.starts_with("viins\x1e"));
        let viins_styled = viins_entry.map_or("", |e| &e["viins\x1e".len()..]);
        assert!(
            !viins_styled.is_empty(),
            "viins styled string should not be empty"
        );
        assert!(
            lines.left2.contains(viins_styled),
            "viins styled string should appear in left2: left2={}, viins={}",
            lines.left2,
            viins_styled
        );
    }

    #[test]
    fn test_fish_layout_single_line() {
        let fast = FastOutputs {
            directory: Some("~/.dotfiles".to_owned()),
            character: Some(">".to_owned()),
            ..make_fast_outputs()
        };
        let slow = SlowOutput {
            git: Some("main".to_owned()),
            ..make_slow_output()
        };
        let lines = compose_prompt(&fast, Some(&slow), 80, &fish_config());

        assert!(
            lines.left1.contains("~/.dotfiles"),
            "left1: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains("(main)"),
            "git branch should be wrapped like fish: {}",
            lines.left1
        );
        assert!(lines.left1.contains('>'), "left1: {}", lines.left1);
        assert_eq!(lines.left2, "", "fish layout should not use line2");
    }

    #[test]
    fn test_fish_layout_status_on_line1() {
        let fast = FastOutputs {
            status: Some("1".to_owned()),
            character: Some(">".to_owned()),
            last_exit_code: 1,
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &fish_config());

        assert!(
            lines.left1.contains("[1]"),
            "status should appear before the character: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains("\x1b[31m"),
            "status should be red: {}",
            lines.left1
        );
        assert_eq!(lines.left2, "", "fish layout should not use line2");
    }

    #[test]
    fn test_fish_layout_keeps_line1_custom_modules() {
        let fast = make_fast_outputs();
        let slow = SlowOutput {
            git: Some("main".to_owned()),
            custom_modules: vec![make_toolchain_module("rust", "v1.82.0")],
        };
        let lines = compose_prompt(&fast, Some(&slow), 80, &fish_config());

        assert!(
            lines.left1.contains("v1.82.0"),
            "line1 module should remain available in fish layout: {}",
            lines.left1
        );
        assert_eq!(lines.left2, "", "fish layout should not use line2");
    }

    #[test]
    fn test_fish_layout_preserves_character_when_truncated() {
        let fast = FastOutputs {
            directory: Some("~/dev/work/tip-extra/tipextra-frontend/frontend".to_owned()),
            status: Some("1".to_owned()),
            character: Some(">".to_owned()),
            last_exit_code: 1,
            custom_modules: vec![make_toolchain_module("rust", "v1.82.0")],
            ..make_fast_outputs()
        };
        let slow = SlowOutput {
            git: Some("hotfix/1660-hide-lcc-link-civil-construction-dev".to_owned()),
            custom_modules: vec![],
        };
        let lines = compose_prompt(&fast, Some(&slow), 40, &fish_config());

        assert!(
            lines.left1.contains("[1]"),
            "status should remain in truncated fish prompt: {}",
            lines.left1
        );
        assert!(
            lines.left1.contains('>'),
            "character should remain in truncated fish prompt: {}",
            lines.left1
        );
        assert!(
            !lines.left1.contains("v1.82.0"),
            "optional modules should be dropped before status/character: {}",
            lines.left1
        );
        assert!(
            display_width(&lines.left1) <= 40,
            "fish prompt should fit requested columns: width={}, line={}",
            display_width(&lines.left1),
            lines.left1
        );
    }

    #[test]
    fn test_fish_char_meta_viins_matches_left1() {
        let fast = FastOutputs {
            character: Some(">".to_owned()),
            ..make_fast_outputs()
        };
        let lines = compose_prompt(&fast, None, 80, &fish_config());
        let viins_entry = lines
            .char_meta
            .split('\x1f')
            .find(|entry| entry.starts_with("viins\x1e"));
        let viins_styled = viins_entry.map_or("", |entry| &entry["viins\x1e".len()..]);

        assert!(
            !viins_styled.is_empty(),
            "viins styled string should not be empty"
        );
        assert!(
            lines.left1.contains(viins_styled),
            "viins styled string should appear in left1: left1={}, viins={}",
            lines.left1,
            viins_styled
        );
    }
}
