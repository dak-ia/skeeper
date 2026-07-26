use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use anyhow::{Context, Result};
use clap::{Command, CommandFactory, Parser, Subcommand};
use clap_mangen::Man;
use skeeper::cli::Cli;

/// docs/man/skeeper.1 の相対path。CWDに依存しないようMANIFEST_DIRから解決する
const MANPAGE_RELATIVE_PATH: &str = "docs/man/skeeper.1";

#[derive(Parser)]
struct XtaskCli {
    #[command(subcommand)]
    command: XtaskCommand,
}

#[derive(Subcommand)]
enum XtaskCommand {
    /// Generate skeeper.1 manpage from the CLI definition and write it to docs/man/skeeper.1
    Manpage,
}

fn main() -> Result<()> {
    let cli = XtaskCli::parse();
    match cli.command {
        XtaskCommand::Manpage => generate_manpage(),
    }
}

fn generate_manpage() -> Result<()> {
    // CARGO_MANIFEST_DIRはxtask/を指す。workspace rootは1階層上
    let workspace_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .context("Failed to locate workspace root")?
        .to_path_buf();
    let dst = workspace_root.join(MANPAGE_RELATIVE_PATH);
    if let Some(parent) = dst.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("Failed to create {}", parent.display()))?;
    }

    let buf = render_manpage().context("Failed to render manpage")?;

    let mut f = File::create(&dst)
        .with_context(|| format!("Failed to open {} for write", dst.display()))?;
    f.write_all(buf.as_bytes())
        .with_context(|| format!("Failed to write {}", dst.display()))?;

    println!("Wrote {}", dst.display());
    Ok(())
}

/// clap_mangenの各render_*_sectionが出力するroffのpreamble(quote定義)。
/// per-section renderのたびに重複するので、出力全体では先頭1回だけ残して他は剥がす
const ESCAPE_PREAMBLE: &str = ".ie \\n(.g .ds Aq \\(aq\n.el .ds Aq '\n";

/// skeeper.1 のroffを組み立てる。default renderだと SUBCOMMANDS が名前だけの目次になるので、
/// 各subcommandの synopsis + options を per-section rendererで inline 展開して単一ファイルに載せる
fn render_manpage() -> Result<String> {
    let cmd = Cli::command();
    // skeeper側のCargo.tomlのversionを`#[command(version)]`が拾っている。xtaskのversionではない
    let version = cmd.get_version().unwrap_or("unknown");
    let source = format!("skeeper {version}");
    let main = Man::new(cmd.clone())
        .source(source)
        .manual("Skeeper Manual");

    let mut buf: Vec<u8> = Vec::new();
    main.render_title(&mut buf)?;
    main.render_name_section(&mut buf)?;
    main.render_synopsis_section(&mut buf)?;
    main.render_description_section(&mut buf)?;
    main.render_options_section(&mut buf)?;

    let top = String::from_utf8(buf).context("clap_mangen wrote non-UTF8 bytes")?;
    let mut out = String::from(ESCAPE_PREAMBLE);
    out.push_str(&strip_all_preamble(&top));

    out.push_str(".SH SUBCOMMANDS\n");
    for sub in cmd.get_subcommands() {
        if sub.is_hide_set() || sub.get_name() == "help" {
            continue;
        }
        append_subcommand_subsection(&mut out, sub)?;
    }

    // clap_mangen が自動生成しない章はここでroffを直書きする
    out.push_str(EXAMPLES_SECTION);
    out.push_str(FILES_SECTION);
    out.push_str(SEE_ALSO_SECTION);

    let mut ver_buf = Vec::new();
    main.render_version_section(&mut ver_buf)?;
    out.push_str(&strip_all_preamble(
        &String::from_utf8(ver_buf).context("clap_mangen wrote non-UTF8 bytes")?,
    ));
    Ok(out)
}

const EXAMPLES_SECTION: &str = r#".SH EXAMPLES
.TP
Create a new session and attach to it:
.EX
skeeper new myproject
.EE
.TP
Create a detached session (do not attach):
.EX
skeeper new -d myproject
.EE
.TP
Attach to an existing session (or open a picker if the name is omitted):
.EX
skeeper attach myproject
.EE
.TP
List sessions, with per-client detail (tty, SSH, attach time):
.EX
skeeper list --detail
.EE
.TP
Detach from the current session (leave the daemon running):
.EX
skeeper detach
.EE
.TP
Kill a session by name and clean up its files:
.EX
skeeper kill myproject
.EE
.TP
Clean up orphan session files after a server crash:
.EX
skeeper prune
.EE
"#;

const FILES_SECTION: &str = r#".SH FILES
.TP
\fI$XDG_RUNTIME_DIR/skeeper/\fR
Runtime directory for session state, created with mode 0700. Falls back to \fI$HOME/.skeeper/run/\fR when \fBXDG_RUNTIME_DIR\fR is unset.
.TP
\fI<uuid>.json\fR
Session metadata: name, cwd, shell, server pid, attached client list. Written atomically via rename(2).
.TP
\fI<uuid>.sock\fR
Unix domain socket that carries pty data between the server and its attached clients.
.TP
\fI<uuid>.ctl\fR
Unix domain socket for control messages (detach request, rename, session switch).
"#;

const SEE_ALSO_SECTION: &str = r#".SH "SEE ALSO"
\fBtmux\fR(1), \fBscreen\fR(1), \fBabduco\fR(1), \fBdtach\fR(1)
"#;

fn append_subcommand_subsection(out: &mut String, sub: &Command) -> Result<()> {
    let display_name = format!("skeeper-{}", sub.get_name());
    out.push_str(".SS ");
    out.push_str(&display_name);
    out.push('\n');
    if let Some(about) = sub.get_about() {
        out.push_str(&about.to_string());
        out.push('\n');
        // .PPで段落break。次のSYNOPSISが同じ行にまとまってしまうのを避ける
        out.push_str(".PP\n");
    }
    // Man renderのSYNOPSISを"skeeper-<sub>"名義にするためCommand::nameを差し替える。
    // clap::Command::nameは`impl Into<Str>`要求で`String`不可なので、xtask寿命の間だけString::leakする
    let renamed = sub.clone().name(display_name.leak() as &'static str);
    let sub_man = Man::new(renamed);
    let mut sub_bytes = Vec::new();
    sub_man.render_synopsis_section(&mut sub_bytes)?;
    sub_man.render_options_section(&mut sub_bytes)?;
    let sub_text = String::from_utf8(sub_bytes).context("clap_mangen wrote non-UTF8 bytes")?;
    // subsection内では `.SH SYNOPSIS`/`.SH OPTIONS` の同格見出しは邪魔なのでstrip。
    // preamble ".ie/.el" も先頭の1回で十分なので削る
    for line in sub_text.lines() {
        if is_escape_preamble(line) || line.starts_with(".SH ") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    Ok(())
}

fn strip_all_preamble(text: &str) -> String {
    let mut result = String::new();
    for line in text.lines() {
        if is_escape_preamble(line) {
            continue;
        }
        result.push_str(line);
        result.push('\n');
    }
    result
}

fn is_escape_preamble(line: &str) -> bool {
    line.starts_with(".ie \\n(.g .ds Aq") || line.starts_with(".el .ds Aq")
}
