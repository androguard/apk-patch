use std::path::PathBuf;
use std::process::ExitCode;

use apk_patch_core::{
    build_project, decode_apk, default_agent_so_path, inject_goauld, BuildOptions, DecodeOptions,
    InjectGoauldOptions, ResResolveMode, VERSION,
};
use apk_patch_framework::{
    clean_frameworks, install_framework, list_frameworks, publicize_resources_file,
    FrameworkOptions,
};
use apk_patch_sign::BuildSignConfig;
use clap::{Parser, Subcommand};
use log::{error, info, LevelFilter};

#[derive(Parser, Debug)]
#[command(name = "apk-patch", version = VERSION, about = "Pure Rust APK pack/unpack/patch tool")]
struct Cli {
    #[arg(short, long, global = true)]
    quiet: bool,

    #[arg(short, long, global = true)]
    verbose: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Decode an APK to a project directory
    #[command(visible_alias = "d")]
    Decode {
        #[arg(value_name = "APK")]
        apk: PathBuf,

        #[arg(short, long)]
        force: bool,

        #[arg(short, long)]
        output: Option<PathBuf>,

        #[arg(short = 's', long = "no-src")]
        no_src: bool,

        #[arg(short = 'r', long = "no-res")]
        no_res: bool,

        #[arg(short = 'a', long = "all-src")]
        all_src: bool,

        #[arg(short = 'j', long = "jobs", default_value_t = default_jobs())]
        jobs: usize,

        #[arg(short = 'p', long = "frame-path")]
        frame_path: Option<PathBuf>,

        #[arg(short = 't', long = "frame-tag")]
        frame_tag: Option<String>,

        #[arg(long = "no-debug-info")]
        no_debug_info: bool,

        #[arg(long = "no-assets")]
        no_assets: bool,

        #[arg(long = "only-manifest")]
        only_manifest: bool,

        #[arg(long = "keep-broken-res")]
        keep_broken_res: bool,

        /// Resource reference resolve mode: default | greedy | lazy
        #[arg(long = "res-resolve-mode", default_value = "default", value_parser = ["default", "greedy", "lazy"])]
        res_resolve_mode: String,

        /// Skip pretty-printing unknown/raw Res_value dumps
        #[arg(long = "ignore-raw-values")]
        ignore_raw_values: bool,
    },

    /// Build a project directory into a signed APK
    #[command(visible_alias = "b")]
    Build {
        #[arg(value_name = "DIR", default_value = ".")]
        dir: PathBuf,

        #[arg(short, long)]
        force: bool,

        #[arg(short, long)]
        output: Option<PathBuf>,

        #[arg(short = 'j', long = "jobs", default_value_t = default_jobs())]
        jobs: usize,

        #[arg(short = 'p', long = "frame-path")]
        frame_path: Option<PathBuf>,

        #[arg(short = 't', long = "frame-tag")]
        frame_tag: Option<String>,

        /// Path to aapt2 binary
        #[arg(long = "aapt")]
        aapt: Option<PathBuf>,

        /// Set android:debuggable="true" on <application>
        #[arg(long = "debuggable")]
        debuggable: bool,

        /// Inject permissive network security config
        #[arg(long = "net-sec-conf")]
        net_sec_conf: bool,

        /// Disable PNG crunching (aapt2 --no-crunch)
        #[arg(long = "no-crunch")]
        no_crunch: bool,

        /// Write build/apk/ intermediates only; skip final APK
        #[arg(long = "no-apk")]
        no_apk: bool,

        /// Copy original manifest + META-INF into the APK
        #[arg(long = "copy-original")]
        copy_original: bool,

        /// Prefer aapt2 over the pure-Rust ARSC builder
        #[arg(long = "use-aapt2")]
        use_aapt2: bool,

        /// Rebuild resources.arsc with the pure-Rust builder (off by default)
        #[arg(long = "rebuild-resources")]
        rebuild_resources: bool,

        #[arg(long = "no-sign")]
        no_sign: bool,

        #[arg(long = "v1-signing-enabled", default_value_t = false)]
        v1_signing_enabled: bool,

        #[arg(long = "v2-signing-enabled", default_value_t = true)]
        v2_signing_enabled: bool,

        #[arg(long = "v3-signing-enabled", default_value_t = true)]
        v3_signing_enabled: bool,
    },

    /// Inject goauld agent (.so) + early-load ContentProvider, rebuild & sign
    #[command(name = "inject-goauld")]
    InjectGoauld {
        #[arg(value_name = "APK")]
        apk: PathBuf,

        #[arg(short, long)]
        output: Option<PathBuf>,

        /// Path to libgoauld_agent.so (default: $GOAULD_AGENT_SO or ../arm_goauld/dist/android-arm64/libgoauld_agent.so)
        #[arg(long = "agent", default_value_os_t = default_agent_so_path())]
        agent: PathBuf,

        /// Keep decoded project at DIR (also used as work directory)
        #[arg(long = "keep-project")]
        keep_project: Option<PathBuf>,

        #[arg(short, long)]
        force: bool,

        #[arg(short = 'j', long = "jobs", default_value_t = default_jobs())]
        jobs: usize,

        #[arg(long = "no-sign")]
        no_sign: bool,

        #[arg(long = "v1-signing-enabled", default_value_t = false)]
        v1_signing_enabled: bool,

        #[arg(long = "v2-signing-enabled", default_value_t = true)]
        v2_signing_enabled: bool,

        #[arg(long = "v3-signing-enabled", default_value_t = true)]
        v3_signing_enabled: bool,
    },

    /// Install a framework APK
    #[command(visible_alias = "if", name = "install-framework")]
    InstallFramework {
        #[arg(value_name = "APK")]
        apk: PathBuf,

        #[arg(short = 'p', long = "frame-path")]
        frame_path: Option<PathBuf>,

        #[arg(short = 't', long = "frame-tag")]
        frame_tag: Option<String>,
    },

    /// Clean installed frameworks
    #[command(visible_alias = "cf", name = "clean-frameworks")]
    CleanFrameworks {
        #[arg(short = 'p', long = "frame-path")]
        frame_path: Option<PathBuf>,

        #[arg(short = 't', long = "frame-tag")]
        frame_tag: Option<String>,

        #[arg(short = 'a', long = "all")]
        all: bool,
    },

    /// List installed frameworks
    #[command(visible_alias = "lf", name = "list-frameworks")]
    ListFrameworks {
        #[arg(short = 'p', long = "frame-path")]
        frame_path: Option<PathBuf>,

        #[arg(short = 't', long = "frame-tag")]
        frame_tag: Option<String>,

        #[arg(short = 'a', long = "all")]
        all: bool,
    },

    /// Publicize resources.arsc entries
    #[command(visible_alias = "pr", name = "publicize-resources")]
    PublicizeResources {
        #[arg(value_name = "RESOURCES_ARSC")]
        path: PathBuf,
    },
}

fn default_jobs() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
        .min(8)
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let level = if cli.quiet {
        LevelFilter::Error
    } else if cli.verbose {
        LevelFilter::Debug
    } else {
        LevelFilter::Info
    };
    let _ = simple_logger::SimpleLogger::new().with_level(level).init();

    match run(cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            error!("E: {e:#}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> anyhow::Result<()> {
    match cli.command {
        Commands::Decode {
            apk,
            force,
            output,
            no_src,
            no_res,
            all_src,
            jobs,
            frame_path,
            frame_tag,
            no_debug_info,
            no_assets,
            only_manifest,
            keep_broken_res,
            res_resolve_mode,
            ignore_raw_values,
        } => {
            let res_resolve_mode = match res_resolve_mode.as_str() {
                "greedy" => ResResolveMode::Greedy,
                "lazy" => ResResolveMode::Lazy,
                _ => ResResolveMode::Default,
            };
            let options = DecodeOptions {
                force,
                output,
                no_src,
                no_res,
                no_assets,
                all_src,
                no_debug_info,
                only_manifest,
                keep_broken_res,
                res_resolve_mode,
                ignore_raw_values,
                frame_path,
                frame_tag,
                jobs,
            };
            let result = decode_apk(&apk, &options)?;
            info!(
                "I: decoded {} entries ({} dex classes) to {}",
                result.entry_count,
                result.dex_class_count,
                result.output_dir.display()
            );
        }
        Commands::Build {
            dir,
            force,
            output,
            jobs,
            frame_path,
            frame_tag,
            aapt,
            debuggable,
            net_sec_conf,
            no_crunch,
            no_apk,
            copy_original,
            use_aapt2,
            rebuild_resources,
            no_sign,
            v1_signing_enabled,
            v2_signing_enabled,
            v3_signing_enabled,
        } => {
            let options = BuildOptions {
                force,
                output,
                jobs,
                aapt,
                frame_path,
                frame_tag,
                debuggable,
                net_sec_conf,
                no_crunch,
                no_apk,
                copy_original,
                skip_aapt2: false,
                use_aapt2,
                rebuild_resources,
                sign: BuildSignConfig {
                    enabled: !no_sign,
                    v1: v1_signing_enabled,
                    v2: v2_signing_enabled,
                    v3: v3_signing_enabled,
                    keystore: None,
                },
            };
            let result = build_project(&dir, &options)?;
            if result.incremental_skipped {
                info!("I: skipped rebuild (up-to-date): {}", result.output_apk.display());
            } else {
                info!(
                    "I: built {} entries -> {} (signed={}, rust_arsc={}, aapt2={})",
                    result.entry_count,
                    result.output_apk.display(),
                    result.signed,
                    result.used_rust_arsc,
                    result.used_aapt2
                );
            }
        }
        Commands::InjectGoauld {
            apk,
            output,
            agent,
            keep_project,
            force,
            jobs,
            no_sign,
            v1_signing_enabled,
            v2_signing_enabled,
            v3_signing_enabled,
        } => {
            let options = InjectGoauldOptions {
                agent_so: agent,
                output,
                force,
                work_dir: keep_project,
                jobs,
                sign: BuildSignConfig {
                    enabled: !no_sign,
                    v1: v1_signing_enabled,
                    v2: v2_signing_enabled,
                    v3: v3_signing_enabled,
                    keystore: None,
                },
            };
            let out = inject_goauld(&apk, &options)?;
            info!("I: goauld-injected APK: {}", out.display());
        }
        Commands::InstallFramework {
            apk,
            frame_path,
            frame_tag,
        } => {
            let options = FrameworkOptions {
                frame_path,
                tag: frame_tag,
                all_tags: false,
            };
            let out = install_framework(&apk, &options)?;
            info!("I: Framework installed to: {}", out.display());
        }
        Commands::CleanFrameworks {
            frame_path,
            frame_tag,
            all,
        } => {
            let options = FrameworkOptions {
                frame_path,
                tag: frame_tag,
                all_tags: all,
            };
            let removed = clean_frameworks(&options)?;
            for path in &removed {
                info!("I: Removing framework file: {}", path.display());
            }
            info!("I: removed {} framework file(s)", removed.len());
        }
        Commands::ListFrameworks {
            frame_path,
            frame_tag,
            all,
        } => {
            let options = FrameworkOptions {
                frame_path,
                tag: frame_tag,
                all_tags: all,
            };
            let listed = list_frameworks(&options)?;
            for path in listed {
                info!("I: {}", path.display());
            }
        }
        Commands::PublicizeResources { path } => {
            publicize_resources_file(&path)?;
            info!("I: Publicized resources: {}", path.display());
        }
    }
    Ok(())
}
