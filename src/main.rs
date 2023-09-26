use gstreamer::prelude::*;
use std::{time::Duration, env, path::Path, process, thread, sync::{Arc, Mutex}};
use log;
use env_logger;

fn main() {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 5 {
        eprintln!("Usage: {} <input video path> <output path> <video encoder> <audio encoder>", args[0]);
        process::exit(1);
    }

    log::info!("Start init gstreamer");
    gstreamer::init().unwrap();

    let input_path = &args[1];
    let output_path = &args[2];
    let video_encoder = &args[3];
    let audio_encoder = &args[4];

    let pipeline_str = format!(
        "filesrc location={} ! qtdemux name=demux \
        demux.video_0 ! decodebin ! videoconvert name=vconv ! {} name=venc ! mux. \
        demux.audio_0 ! decodebin ! audioconvert name=aconv ! {} name=aenc ! mux. \
        {} name=mux ! filesink location={}",
        input_path,
        get_muxer_from_extension(&Path::new(output_path).extension().unwrap().to_string_lossy()), output_path,
        video_encoder,
        audio_encoder,
    );


    log::info!("Start parse launch pipeline: {:}", pipeline_str);
    let pipeline = Arc::new(Mutex::new(gstreamer::parse_launch(&pipeline_str).unwrap()));

/*
    {
        let pipeline = pipeline.lock().unwrap().clone();
        if let Some(dec_el) = pipeline.dynamic_cast::<gstreamer::Bin>().unwrap().by_name("fix") {
            dec_el.connect("handoff", false, move |args| {
                // let identity = args[0].get::<gstreamer::Element>().unwrap();
                let buffer = args[1].get::<gstreamer::buffer::Buffer>().unwrap();
                println!("BUFFER={:?}", buffer);

                // DTSやPTSを取得して変更する
                // let dts = buffer.dts();
                let pts = buffer.pts();
                let dts = buffer.dts();
                println!("PTS={:?} DTS={:?}", pts, dts);

                None
            });
        }
    }

    {
        dec_el.connect_pad_added(|_, pad| {
            let caps = pad.current_caps().unwrap();
            let structures = caps.iter().collect::<Vec<_>>();
            assert!(structures.len() == 1);
            if pad.direction() == gstreamer::PadDirection::Src {
                if structures[0].name().to_string().starts_with("video") {
                    pad.add_probe(gstreamer::PadProbeType::BUFFER_LIST, |_, info| {
                        if let Some(gstreamer::PadProbeData::BufferList(buffers)) = &info.data {
                            for data in buffers.iter() {
                                let pts = data.pts();
                                let dts = data.dts();
                                println!("BUFFERLIST EL PTS: {:?}, DTS: {:?}", pts, dts);
                            }
                        }

                        gstreamer::PadProbeReturn::Ok
                    });
                    pad.add_probe(gstreamer::PadProbeType::BUFFER, |_, info| {
                        if let Some(gstreamer::PadProbeData::Buffer(data)) = &mut info.data {
                            let pts = data.pts();
                            let dts = data.dts();
                            if dts == None && pts < dts {
                                data.get_mut().unwrap().set_dts(pts);
                            }
                            println!("PTS: {:?}, DTS: {:?}", pts, dts);
                        }

                        gstreamer::PadProbeReturn::Ok
                    });
                }
            }
        });
    };
*/

    let pipeline_weak = Arc::downgrade(&pipeline);

    thread::spawn(move || loop {
        thread::sleep(Duration::from_secs(1));

        if let Some(pipeline) = pipeline_weak.upgrade() {
            let pipeline = pipeline.lock().unwrap();
            if let Some(position) = pipeline.query_position::<gstreamer::ClockTime>() {
                if let Some(duration) = pipeline.query_duration::<gstreamer::ClockTime>() {
                    println!("Position: {} / {}", position.display(), duration.display());
                }
            }
        };
    });

    log::info!("Set state to playing");
    let bus = {
        let pipeline = pipeline.lock().unwrap();
        pipeline.set_state(gstreamer::State::Playing).unwrap();
        pipeline.bus().unwrap()
    };

    log::info!("Before message loop");
    for msg in bus.iter_timed(gstreamer::ClockTime::NONE) {
        match msg.view() {
            gstreamer::MessageView::Eos(..) => break,
            gstreamer::MessageView::Error(err) => {
                eprintln!(
                    "Error from {:?}: {} ({:?})",
                    msg.src().map(|s| s.path_string()),
                    err.error(),
                    err.debug()
                );
                process::exit(1);
            }
            _ => (),
        }
    }
    log::info!("After message loop");

    pipeline.lock().unwrap().set_state(gstreamer::State::Null).unwrap();
}

fn get_muxer_from_extension(ext: &str) -> &'static str {
    match ext {
        "mp4" => "mp4mux",
        "mkv" => "matroskamux",
        "avi" => "avimux",
        _ => {
            eprintln!("Unsupported format: {}", ext);
            process::exit(1);
        }
    }
}

