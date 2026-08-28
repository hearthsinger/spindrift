use anyhow::Error;
use std::fs::{read_dir, File};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::thread;

use clap::Parser;

use droplet::Droplet;
use spindrift::Spindrift;
use tera::Tera;

mod droplet;
mod errors;
mod spindrift;
mod templ;

#[derive(Debug, Parser)]
#[clap(author, version, about)]
struct Args {
    #[clap(short, long = "config", parse(from_os_str), default_value="spindrift.yaml")]
    config_file: PathBuf,
    #[clap(short, long, parse(from_os_str))]
    /// The path to the directory containing the valid yaml droplets
    source_dir: PathBuf,
    #[clap(short, long, parse(from_os_str))]
    /// The path where the rendered html should be written
    out_dir: PathBuf,
    #[clap(short, long, default_value = "templates")]
    /// The directory containing valid tera templates
    template_dir: String,
    #[clap(short, long, default_values=&["yaml", "yml"])]
    /// Supported file extensions for droplets, excluding the preceeding '.'
    extensions: Vec<String>,
}

fn main() -> Result<(), Error> {
    let found_files = Arc::new(Mutex::new(0));

    let mut ignored_files = 0;
    let args = Args::parse();

    let spindrift = match Spindrift::from_file(args.config_file.as_path()) {
        Ok(s) => s,
        Err(why) => {
            println!("! Error parsing spindrift project config: {:?}", why);
            ::std::process::exit(1);
        }
    };

    println!("> Spindrift project config: {:#?}", spindrift);

    let mut use_templates = args.template_dir.clone();
    use_templates.push_str("/*.html");
    println!("> using templates: {}", use_templates);
    let templates: Arc<Mutex<Tera>> = {
        let mut tera = match Tera::new(&use_templates) {
            Ok(t) => t,
            Err(why) => {
                println!("! Failed to parse templates:\n{}", why);
                ::std::process::exit(1);
            }
        };
        tera.autoescape_on(vec![]);
        tera.register_filter("batch", templ::BatchFilter);

        Arc::new(Mutex::new(tera))
    };

    let mut droplets: Vec<Droplet> = Vec::new();
    let (tx, rx) = mpsc::channel();

    match read_dir(args.source_dir) {
        Err(why) => println!("! {:?}", why.kind()),
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path_buff = entry.path();

                    if let Some(file_extension) = path_buff.extension() {
                        if !args
                            .extensions
                            .contains(&file_extension.to_string_lossy().into_owned())
                        {
                            println!(
                                "> Ignoring\t{:?} - missing or bad file extension",
                                path_buff
                            );
                            ignored_files += 1;
                            continue;
                        }
                    } else {
                        println!("> Found\t\t{:?}", path_buff);
                    }

                    let (thread_ctr, thread_tx) = (Arc::clone(&found_files), tx.clone());
                    thread::spawn(move || {
                        println!(
                            "({:?})> Starting processing {:?}",
                            thread::current().id(),
                            path_buff
                        );

                        if !path_buff.is_dir() {
                            let mut ctr = thread_ctr.lock().unwrap();
                            *ctr += 1;
                            thread_tx
                                .send(Droplet::from_file(path_buff.as_path()))
                                .unwrap();
                        } else {
                            println!(
                                "({:?})> {:?} -- ignoring directory (for now)",
                                thread::current().id(),
                                path_buff
                            );
                        }
                    });
                }
            }
            drop(tx);
        }
    }

    let (out_tx, out_rx) = mpsc::channel();

    for received in rx {
        match received {
            Err(why) => {
                println!("! Thread hit error processing file: {:#?}", why);
            }
            Ok(droplet) => {
                println!("> Processed '{}' as {}", droplet.title, droplet.file_name());
                let (thread_out_dir, thread_tmpl, thread_out_tx) = (
                    args.out_dir.join(droplet.file_name()),
                    Arc::clone(&templates),
                    out_tx.clone(),
                );
                thread::spawn(move || {
                    println!(
                        "({:?})> Writing {} to output directory",
                        thread::current().id(),
                        droplet.file_name()
                    );
                    let tmpl = thread_tmpl.lock().unwrap();
                    match tmpl.render_to(
                        "droplet.html",
                        &(droplet.as_context()),
                        File::create(thread_out_dir.clone()).unwrap(),
                    ) {
                        Ok(_) => {
                            println!("> Wrote '{}' to {:?}", droplet.title, thread_out_dir);
                            thread_out_tx.send(Ok(droplet)).unwrap();
                        }
                        Err(e) => {
                            thread_out_tx.send(Err(e)).unwrap();
                        }
                    };
                });
            }
        }
    }
    drop(out_tx);

    for res in out_rx {
        match res {
            Err(why) => {
                println!("! Output thread hit error writing file: {:#?}", why);
            }
            Ok(mut droplet) => {
                droplet.set_file_name();
                droplets.push(droplet);
            }
        }
    }

    let total_files: u32 = *(found_files.lock().unwrap());
    drop(found_files);
    println!(
        "> Finished processing pages!\n\n\
                  \tIgnored pages:\t\t\t\t{}\n\
                  \tFailed pages:\t\t\t\t{}\n\
                  \tProcessed pages:\t\t\t{}\n\
                  \tTotal scanned (excl. directories):\t{}\n",
        ignored_files,
        total_files - droplets.len() as u32,
        droplets.len(),
        total_files + ignored_files,
    );

    let tmpl = templates.lock().unwrap();
    let index_out_dir = args.out_dir.join("index.html");
    match tmpl.render_to(
        "spindrift.html",
        &(spindrift.as_context(droplets)),
        File::create(index_out_dir.clone()).unwrap(),
    ) {
        Ok(_) => println!("> Wrote page index out to {:?}", index_out_dir),
        Err(why) => println!("! Failed to write index page {:#?}", why),
    };

    Ok(())
}
