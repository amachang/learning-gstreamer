use std::{env, path::Path, fs::File, io::{Cursor, BufReader, Read, Seek, SeekFrom}, str::FromStr};

use byteorder::{BigEndian, ReadBytesExt};
use h264_reader::{nal, nal::{Nal, RefNal}};

fn main() -> mp4::Result<()> {
    let args = env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        panic!("Usage: {} <h264_isomp4_file_path>", args[0]);
    }
    let path = Path::new(&args[1]);
    let file = File::open(path).unwrap();
    let size = file.metadata().unwrap().len();

    let mut reader = BufReader::new(file);

    debug_box_hex(&mut reader, size, "".to_string())?;


    let file = File::open(path).unwrap();
    let size = file.metadata().unwrap().len();
    let reader = BufReader::new(file);
    let mp4 = mp4::Mp4Reader::read_header(reader, size)?;

    let video_traks = mp4.moov.traks.iter().filter(|t| t.mdia.hdlr.handler_type == mp4::FourCC::from_str("vide").unwrap()).collect::<Vec<_>>();
    assert_eq!(video_traks.len(), 1); // TODO
    let video_trak = video_traks[0];
    let Some(avc1) = &video_trak.mdia.minf.stbl.stsd.avc1 else {
        panic!("Not a h264 codec");
    };
    let nal_size_length: usize = (avc1.avcc.length_size_minus_one + 1).into();

    let video_stbl = &video_trak.mdia.minf.stbl;
    let stsc = &video_stbl.stsc;
    let stsz = &video_stbl.stsz;

    let file = File::open(path).unwrap();
    let mut reader = BufReader::new(file);

    match (&video_stbl.stco, &video_stbl.co64) {
        (Some(stco), None) => {
            let chunk_count = stco.entries.len();
            let mut stsc_entries = stsc.entries.iter().peekable();
            loop {
                let (chunk_end_index, sample_count, last) = match (stsc_entries.next(), stsc_entries.peek()) {
                    (Some(entry), Some(next_entry)) => (next_entry.first_chunk.checked_sub(1).unwrap() as usize, entry.samples_per_chunk, false),
                    (Some(entry), None) => (chunk_count, entry.samples_per_chunk, true),
                    _ => unreachable!(),
                };
                let mut total_sample_index = 0;
                let mut chunk_index = 0;
                while chunk_index < chunk_end_index {
                    let chunk_offset = stco.entries[chunk_index];

                    let mut sample_offset = chunk_offset;
                    for _ in 0..sample_count {
                        let sample_size = if 0 < stsz.sample_size {
                            assert_eq!(stsz.sample_sizes.len(), 0);
                            stsz.sample_size
                        } else {
                            stsz.sample_sizes[total_sample_index]
                        };

                        reader.seek(SeekFrom::Start(sample_offset.into()))?;
                        let mut buf = vec![0u8; sample_size.try_into().unwrap()];
                        reader.read_exact(&mut buf)?;

                        let mut cursor = Cursor::new(&buf);
                        let nal_size: usize = cursor.read_uint::<BigEndian>(nal_size_length)? as usize;

                        let nal = RefNal::new(&buf[nal_size_length..(nal_size_length + nal_size)], &[], true);
                        assert!(nal.is_complete());
                        if nal.header().unwrap().nal_unit_type() == nal::UnitType::SEI {
                            /*
                            let mut scratch = Vec::new();
                            let mut sei_reader = nal::sei::SeiReader::from_rbsp_bytes(nal.rbsp_bytes(), &mut scratch);
                            while let Ok(Some(msg)) = sei_reader.next() {
                                if msg.payload_type == nal::sei::HeaderType::PicTiming {
                                    println!("    SEI Message: {:?}", msg);
                                } else {
                                    println!("    SEI Message: {:?}", msg);
                                }
                            }
                            */
                        } else {
                            println!("Sample {:03}: {} + {}: {:?}", chunk_index, sample_offset, sample_size, nal.header().unwrap().nal_unit_type());
                        }

                        // debug_hex(buf, "    ");

                        sample_offset += sample_size;
                        total_sample_index += 1;
                    }
                    chunk_index += 1;
                };
                if last {
                    break;
                }
            }
        },
        (None, Some(_co64)) => {
            todo!();
        },
        _ => {
            panic!("Invalid chunk offset block");
        },
    };

    /*
    let (track_id, _) = mp4.tracks().iter().find(|(_, track)| match track.track_type() {
        Ok(mp4::TrackType::Video) => true,
        _ => false,
    }).expect("Not found video stream");

    println!("{}", mp4.sample_count(*track_id).unwrap());
    */ 

    Ok(())
}

fn debug_box_hex<R: Read + Seek>(reader: &mut BufReader<R>, size: u64, indent: String) -> mp4::Result<()> {
    while reader.stream_position()? < size {
        let header_start_pos = reader.stream_position()?;
        let header = mp4::BoxHeader::read(reader)?;
        let header_end_pos = reader.stream_position()?;

        if header.size == 0 {
            break;
        };

        let header_itself_size = header_end_pos - header_start_pos;
        let body_size = header.size - header_itself_size;

        reader.seek(SeekFrom::Start(header_start_pos))?;

        let (header_bytes, truncated) = read(reader, header_itself_size);
        reader.seek(SeekFrom::Start(header_end_pos))?;

        println!("{}{:?}", indent, header);
        debug_hex(header_bytes, &indent);
        if truncated { println!("{}...", indent) }

        match header.name {
            mp4::BoxType::MoovBox | mp4::BoxType::TrakBox | mp4::BoxType::MdiaBox | mp4::BoxType::MinfBox | mp4::BoxType::StblBox => {
                debug_box_hex(reader, size, indent.clone() + "    ")?;
            },
            _ => {
                let (body_bytes, truncated) = read(reader, body_size);

                println!("{}BODY ({})", indent, body_size);
                debug_hex(body_bytes, &indent);
                if truncated { println!("{}...", indent) }
                println!("\n");
            },
        }
        reader.seek(SeekFrom::Start(header_end_pos + body_size))?;
    };
    println!("\n");
    Ok(())
}

fn read<R: Read>(reader: &mut BufReader<R>, size: u64) -> (Vec<u8>, bool) {
    let max_size = 128;
    let (truncated, size) = if max_size < size { (true, max_size) } else { (false, size) };
    let mut buf = vec![0u8; size as usize];
    reader.read_exact(&mut buf).unwrap();
    (buf, truncated)
}

fn debug_hex(buf: Vec<u8>, indent: &str) {
    let mut s = String::new();
    let cols = 16;
    for row in 0..(buf.len() / cols + 1) {
        s.push_str(&format!("{}{:04x}::", indent, row * cols));
        for col in 0..cols {
            let index = row * cols + col;
            if index < buf.len() {
                s.push_str(&format!(" {:02x}", buf[index]));
            }
        }
        s.push_str("\n");
    }
    println!("{}", s);
}

