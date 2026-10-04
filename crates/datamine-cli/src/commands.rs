//! One handler per subcommand. Handlers parse arguments into core calls
//! and hand results to `render`; no business logic lives here.

use std::io::Write;

use anyhow::{Context, Result, bail};
use datamine_core::db::RecordFilter;
use datamine_core::facets::{self, FacetFilter};
use datamine_core::import::{ImportOptions, import};
use datamine_core::sp::{self, SpRules};
use datamine_core::wz::{self, JsonOptions, WzTree};
use datamine_core::{Store, catalog, diff, extract, jobs};

use crate::cli::{Cli, Command, Format};
use crate::render;

pub fn run(cli: Cli) -> Result<()> {
    if let Command::Serve { addr } = cli.command {
        return datamine_web::serve(&cli.store, addr);
    }
    let mut store = Store::open(&cli.store)?;
    match cli.command {
        Command::Import {
            client,
            label,
            patchdata,
            channel,
            note,
            baseline,
            no_extract,
        } => {
            let report = import(
                &mut store,
                &ImportOptions {
                    client_dir: client,
                    patchdata_dir: patchdata,
                    label,
                    channel,
                    note,
                },
            )?;
            println!(
                "imported {} ({} files, {}, {} new)",
                report.version.label,
                report.files,
                render::bytes(report.bytes),
                render::bytes(report.new_bytes),
            );
            if baseline && !report.became_baseline {
                store.set_baseline(&report.version)?;
            }
            if baseline || report.became_baseline {
                println!("{} is now the baseline", report.version.label);
            }
            if !no_extract {
                let summaries = extract::run(&mut store, &report.version, &[])?;
                render::extract_summaries(&summaries);
            }
        }

        Command::Versions => {
            let baseline = store.baseline()?.map(|v| v.id);
            render::versions(&store.db().versions()?, baseline);
        }

        Command::Remove { version, yes } => {
            let v = store.resolve(&version)?;
            if !yes {
                println!(
                    "would remove {} (#{}) and {}; rerun with --yes",
                    v.label,
                    v.id,
                    store.snapshot_dir(&v).display()
                );
                return Ok(());
            }
            store.remove_version(&v)?;
            println!("removed {}; run `datamine gc` to free disk space", v.label);
        }

        Command::Gc => {
            let report = store.gc()?;
            println!(
                "removed {} files ({}) and {} blobs",
                report.objects,
                render::bytes(report.bytes),
                report.blobs
            );
        }

        Command::Baseline { version } => match version {
            Some(sel) => {
                let v = store.resolve(&sel)?;
                store.set_baseline(&v)?;
                println!("baseline set to {}", v.label);
            }
            None => match store.baseline()? {
                Some(v) => println!("{}", v.label),
                None => println!("no baseline set"),
            },
        },

        Command::Info { version } => {
            let v = store.resolve(&version)?;
            render::info(
                &v,
                &store.db().extractions(v.id)?,
                &store.db().kind_counts(v.id)?,
            );
        }

        Command::Extractors => {
            for e in extract::registry() {
                println!("{:<10} {}", e.name(), e.description());
            }
        }

        Command::Extract { version, only } => {
            let v = store.resolve(&version)?;
            let summaries = extract::run(&mut store, &v, &only)?;
            render::extract_summaries(&summaries);
        }

        Command::Diff {
            from,
            to,
            kind,
            category,
            job,
            data_only,
            hide_text,
            format,
            out,
            limit,
        } => {
            let mut facets = FacetFilter {
                data_only,
                ..Default::default()
            };
            for c in &category {
                facets.add_category(c).map_err(anyhow::Error::msg)?;
            }
            for j in &job {
                facets.add_job(j).map_err(anyhow::Error::msg)?;
            }
            let to = store.resolve(&to)?;
            let from = match &from {
                Some(sel) => store.resolve(sel)?,
                None => store.diff_base(&to)?.with_context(|| {
                    format!("nothing to compare: {} is the oldest version", to.label)
                })?,
            };
            if from.id == to.id {
                bail!(
                    "--from and --to are both {}; import another version first",
                    from.label
                );
            }
            let filter = RecordFilter {
                kind: kind.as_deref(),
            };
            let mut cs = diff::changeset(&store, &from, &to, &filter)?;
            cs.retain(&facets);
            if hide_text {
                cs.hide_text_only();
            }
            let text = match format {
                Format::Text => render::changeset_text(&cs, limit),
                Format::Markdown => render::changeset_markdown(&cs, limit),
                Format::Json => serde_json::to_string_pretty(&cs)?,
            };
            emit(out.as_deref(), &text)?;
        }

        Command::History { key, kind, json } => {
            let filter = RecordFilter {
                kind: kind.as_deref(),
            };
            let hist = diff::history(&store, &key, &filter)?;
            if json {
                println!("{}", serde_json::to_string_pretty(&hist)?);
            } else if hist.is_empty() {
                println!("no records match {key:?}");
            } else {
                print!("{}", render::history_text(&hist));
            }
        }

        Command::Search {
            text,
            version,
            kind,
            limit,
        } => {
            let v = store.resolve(&version)?;
            let filter = RecordFilter {
                kind: kind.as_deref(),
            };
            let hits = store.db().search(v.id, &text, &filter, limit)?;
            render::search(&hits, limit);
        }

        Command::Ls { version, path } => {
            let v = store.resolve(&version)?;
            let tree = WzTree::open(&store.snapshot_dir(&v))?;
            let node = tree.get(&path)?;
            let mut children = wz::children(&node);
            children.sort_by(|a, b| wz::natural_cmp(&a.0, &b.0));
            for (name, child) in children {
                let preview = if wz::is_value(&child) {
                    render::preview(&wz::node_to_json(&child, JsonOptions::default()), 80)
                } else {
                    wz::display_name(&child).unwrap_or_default()
                };
                println!("{:<8} {name}  {preview}", wz::type_name(&child));
            }
        }

        Command::Show {
            version,
            path,
            depth,
        } => {
            let v = store.resolve(&version)?;
            let tree = WzTree::open(&store.snapshot_dir(&v))?;
            let node = tree.get(&path)?;
            wz::parse_to_depth(&node, depth)?;
            let opts = JsonOptions {
                content_hashes: false,
                max_depth: (depth > 0).then_some(depth),
            };
            println!(
                "{}",
                serde_json::to_string_pretty(&wz::node_to_json(&node, opts))?
            );
        }

        Command::Serve { .. } => unreachable!("handled above"),

        Command::Jobs => {
            for j in jobs::JOBS {
                println!(
                    "{:<5} {:<28} {:<9} {}",
                    j.id,
                    j.name,
                    j.branch.name(),
                    adv_name(j.advancement)
                );
            }
        }

        Command::Skills { job, version, all } => {
            let v = store.resolve(&version)?;
            let mut facets = FacetFilter::default();
            if let Some(j) = &job {
                facets.add_job(j).map_err(anyhow::Error::msg)?;
            }
            for s in catalog::skills(&store, v.id)? {
                let f = facets::of("skill", &s.id, None);
                if !facets.matches(&f) || !(all || s.is_learnable()) {
                    continue;
                }
                let req: Vec<_> = s.req.iter().map(|(id, lv)| format!("{id}@{lv}")).collect();
                println!(
                    "{:<8} {:<22} {:<28} max {:>2}  {}",
                    s.id,
                    jobs::name(s.job),
                    s.label(),
                    s.max_level,
                    if req.is_empty() {
                        String::new()
                    } else {
                        format!("needs {}", req.join(", "))
                    }
                );
            }
        }

        Command::Skill { id, version, json } => {
            let v = store.resolve(&version)?;
            let skill = catalog::skill(&store, v.id, &id)?
                .with_context(|| format!("no skill {id} in {}", v.label))?;
            if json {
                println!("{}", serde_json::to_string_pretty(&skill)?);
            } else {
                print!("{}", render::skill(&skill));
            }
        }

        Command::Sp {
            job,
            level,
            build,
            version,
        } => {
            let target = jobs::find(&job).with_context(|| format!("unknown job {job:?}"))?;
            let rules = SpRules::default();
            let build = match &build {
                Some(b) => sp::parse_build(b).map_err(anyhow::Error::msg)?,
                None => Default::default(),
            };
            let v = store.resolve(&version)?;
            let skills = catalog::skills(&store, v.id)?;
            let eval = sp::evaluate(&rules, target, level, &build, &skills);
            print!("{}", render::sp(&eval, target, level));
        }

        Command::ExportImage { version, path, out } => {
            let v = store.resolve(&version)?;
            let tree = WzTree::open(&store.snapshot_dir(&v))?;
            let node = tree.get(&path)?;
            wz::canvas_image(&node)?
                .save(&out)
                .with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.display());
        }
    }
    Ok(())
}

fn emit(out: Option<&std::path::Path>, text: &str) -> Result<()> {
    match out {
        Some(path) => {
            std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))?;
            eprintln!("wrote {}", path.display());
        }
        None => {
            let mut stdout = std::io::stdout().lock();
            stdout.write_all(text.as_bytes())?;
            if !text.ends_with('\n') {
                writeln!(stdout)?;
            }
        }
    }
    Ok(())
}

fn adv_name(advancement: u8) -> &'static str {
    match advancement {
        0 => "beginner",
        1 => "1st job",
        2 => "2nd job",
        3 => "3rd job",
        _ => "4th job",
    }
}
