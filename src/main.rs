use clap::Parser;
use std::fs::File;
use std::io::prelude::*;
use std::io::Cursor;
use std::io::SeekFrom;
use std::process::exit;
use std::usize;
use binrw::{binrw, BinRead, BinWrite};

#[derive(Parser, Debug)]
#[command()]
struct Args {
    #[arg()]
    scp_in: Vec<String>,

    #[arg(short('o'))]
    scp_out: String,

    #[arg(short('t'))]
    tracks: Vec<String>,
}

#[binrw]
#[brw(little, magic=b"SCP")]
#[derive(Debug, Copy, Clone)]
struct ScpHeader {
    version: u8,
    disk_type: u8,
    rev_count: u8,
    start_track: u8,
    end_track: u8,
    flags: u8,
    bitcell_time: u8,
    heads: u8,
    resolution: u8,
    checksum: u32,
    track_data_headers: [u32; 168],
}

#[binrw]
#[brw(little, magic=b"TRK", import(rev_count: u8))]
#[derive(Debug, Clone)]
struct ScpTrack {
    track_number: u8,
    #[br(count=rev_count)]
    revs: Vec<ScpRev>,
}

#[binrw]
#[brw(little)]
#[derive(Debug, Copy, Clone)]
struct ScpRev {
    duration: u32,
    num_bitcells: u32,
    offset: u32,
}

struct Scp {
    file: File,
    header: ScpHeader,
    tracks: Vec<Option<ScpTrack>>,
}

fn checksum(data: &[u8]) -> u32 {
    let mut sum: u32 = 0;
    for &byte in data {
        sum = sum.wrapping_add(byte as u32);
    }
    sum
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if args.scp_in.len() != 2 {
        eprintln!("Two input scp files must be specified");
        exit(1);
    }
    let mut track_params: Vec<u8> = vec![0; 168];
    for param in args.tracks {
        let split: Vec<_> = param.split(":").collect();
        track_params[split[0].parse::<usize>().unwrap() + split[1].parse::<usize>()?*2] = split[2].parse()?;
    }
    let mut scp_in_files: Vec<_> = args.scp_in.into_iter().map(|in_file| {
        let mut file = File::open(&in_file).unwrap();
        let header = ScpHeader::read(&mut file).unwrap();
        if header.bitcell_time != 0 {
            eprintln!("{in_file}: Unsupported bitcell time");
            exit(1);
        }
        let tracks: Vec<_> = header.track_data_headers.into_iter().map(|offset| {
            if offset != 0 {
                file.seek(SeekFrom::Start(offset as u64)).unwrap();
                let track = ScpTrack::read_args(&mut file, (header.rev_count,)).unwrap();
                Some(track)
            } else {
                None
            }
        }).collect();
        Scp{file, header, tracks}
    }).collect();

    let mut scp_out_header = scp_in_files[0].header.clone();
    scp_out_header.checksum = 0;
    let mut out_file = File::create(args.scp_out)?;
    let mut sum: u32 = 0;
    scp_out_header.write(&mut out_file)?; // initial write, will be updated
    for i in 0..168 {
        if scp_in_files[0].tracks[i].is_none() {
            scp_out_header.track_data_headers[i] = 0;
            continue;
        }
        let source_file = &mut scp_in_files[track_params[i] as usize];
        let mut new_track = (*source_file).tracks[i].clone().unwrap();
        let track_header_pos = out_file.stream_position()?;
        scp_out_header.track_data_headers[i] = track_header_pos as u32;
        new_track.write(&mut out_file)?;
        for (j, rev) in new_track.revs.iter_mut().enumerate() {
            // get flux data from source file
            source_file.file.seek(SeekFrom::Start(source_file.header.track_data_headers[i] as u64
                                                  + (*source_file).tracks[i].clone().unwrap().revs[j].offset as u64))?;
            let mut flux_data = vec![0; source_file.tracks[i].clone().unwrap().revs[j].num_bitcells as usize * 2];
            source_file.file.read_exact(&mut flux_data)?;
            let flux_pos = out_file.stream_position()? - track_header_pos;
            rev.offset = flux_pos as u32;
            sum = sum.wrapping_add(checksum(&flux_data));
            out_file.write_all(&flux_data)?;
        }
        out_file.seek(SeekFrom::Start(track_header_pos))?;
        let mut track_header_data = Cursor::new(Vec::<u8>::new());
        new_track.write(&mut track_header_data)?; // rewrite track
        sum = sum.wrapping_add(checksum(&track_header_data.get_ref()));
        out_file.write_all(&track_header_data.get_ref())?;
        out_file.seek(SeekFrom::End(0)).unwrap();
    }
    let mut header_for_checksum = Cursor::new(Vec::<u8>::new());
    scp_out_header.write(&mut header_for_checksum)?;
    sum = sum.wrapping_add(checksum(&header_for_checksum.get_ref()[0x10..]));
    scp_out_header.checksum = sum;
    out_file.seek(SeekFrom::Start(0))?;
    scp_out_header.write(&mut out_file)?; // rewrite output header
    Ok(())
}
