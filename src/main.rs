use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use dialoguer::{Input, theme::ColorfulTheme};
use indicatif::{HumanBytes, ProgressBar, ProgressStyle};
use parking_lot::Mutex;
use rayon::iter::{IntoParallelRefIterator, ParallelIterator};
use re_tex::tex::Tex;
use ree_pak_core::{filename::FileNameTable, read::archive::PakArchiveReader, write::FileOptions};

const FILE_NAME_LIST: &[u8] = include_bytes!("../assets/MHWs_STM_Release.list.zst");

fn main() {
    println!("Version {}. Tool by @Eigeen", env!("CARGO_PKG_VERSION"));

    if let Err(e) = main_entry() {
        eprintln!("Error: {e}");
        wait_for_exit();
        std::process::exit(1);
    }
}

fn main_entry() -> eyre::Result<()> {
    let input: String = Input::with_theme(&ColorfulTheme::default())
        .show_default(true)
        .default("re_chunk_000.pak.sub_000.pak".to_string())
        .with_prompt("Input .pak file path")
        .interact_text()
        .unwrap();

    println!("Input file: {}", input);
    let input_path = Path::new(&input);
    if !input_path.is_file() {
        eyre::bail!("input file not exists.");
    }

    println!("Loading embedded file name table...");
    let filename_table = FileNameTable::from_bytes(FILE_NAME_LIST)?;

    let file = fs::File::open(input_path)?;
    let mut reader = io::BufReader::new(file);

    println!("Reading pak archive...");
    let pak_archive = ree_pak_core::read::read_archive(&mut reader)?;
    let archive_reader = PakArchiveReader::new(reader, &pak_archive);
    let archive_reader_mtx = Mutex::new(archive_reader);

    // filtered entries
    println!("Filtering entries...");
    let entries = pak_archive
        .entries()
        .iter()
        .filter(|entry| {
            let Some(file_name) = filename_table.get_file_name(entry.hash()) else {
                return false;
            };
            file_name.get_name().ends_with(".tex.241106027")
        })
        .collect::<Vec<_>>();

    // new pak archive
    let output_path = input_path.with_extension("uncompressed.pak");
    println!("Output file: {}", output_path.to_string_lossy());
    let out_file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(output_path)?;
    let pak_writer = ree_pak_core::write::PakWriter::new(out_file, entries.len() as u64);
    let pak_writer_mtx = Arc::new(Mutex::new(pak_writer));

    let bar = ProgressBar::new(entries.len() as u64);
    bar.set_style(
        ProgressStyle::default_bar().template("Bytes written: {msg}\n{pos}/{len} {wide_bar}")?,
    );
    bar.enable_steady_tick(Duration::from_millis(200));

    let pak_writer_mtx1 = Arc::clone(&pak_writer_mtx);
    let bar1 = bar.clone();
    let bytes_written = AtomicUsize::new(0);
    let err = entries
        .par_iter()
        .try_for_each(move |&entry| -> eyre::Result<()> {
            let pak_writer_mtx = &pak_writer_mtx1;
            let bar = &bar1;
            // read raw tex file
            // parse tex file
            let mut entry_reader = {
                let mut archive_reader = archive_reader_mtx.lock();
                archive_reader.owned_entry_reader(entry.clone())?
            };
            let mut tex = Tex::from_reader(&mut entry_reader)?;
            // decompress mipmaps
            tex.batch_decompress()?;

            let tex_bytes = tex.as_bytes()?;
            bytes_written.fetch_add(tex_bytes.len() as usize, Ordering::SeqCst);

            // save file
            let file_name = filename_table.get_file_name(entry.hash()).unwrap().clone();
            {
                let mut pak_writer = pak_writer_mtx.lock();
                // clone attributes from original file
                pak_writer.start_file(
                    file_name,
                    FileOptions::default().with_unk_attr(*entry.unk_attr()),
                )?;
                pak_writer.write_all(&tex_bytes)?;
            }
            bar.inc(1);
            if bar.position() % 100 == 0 {
                bar.set_message(
                    HumanBytes(bytes_written.load(Ordering::SeqCst) as u64).to_string(),
                );
            }
            Ok(())
        });
    if let Err(e) = err {
        eprintln!("Error occurred when processing tex: {e}");
        eprintln!(
            "The process terminated early, we'll save the current processed tex files to pak file."
        );
    }

    let pak_writer = Arc::try_unwrap(pak_writer_mtx);
    match pak_writer {
        Ok(pak_writer) => pak_writer.into_inner().finish()?,
        Err(_) => panic!("Arc::try_unwrap failed"),
    };

    bar.finish();
    println!("Done!");
    println!(
        "You should rename the output file like `re_chunk_000.pak.sub_000.pak.patch_xxx.pak`, or manage it by your favorite mod manager."
    );

    Ok(())
}

fn wait_for_exit() {
    let _: String = Input::with_theme(&ColorfulTheme::default())
        .allow_empty(true)
        .interact_text()
        .unwrap();
}
