use std::{env, path::Path, thread};

use gstreamer as gst;
use gst::prelude::*;

use log;
use env_logger;

fn main() {
    env_logger::init();

    log::debug!("Started main process: {:?}", thread::current().id());

    let args = env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        panic!("Usage: {} <h264_isomp4_file_path>", args[0]);
    }
    let path = Path::new(&args[1]);

    if let Err(err) = gst::init() {
        panic!("Failed to init gstreamer: {}", err);
    }

    let pipeline = gst::Pipeline::builder()
        // 含まれる子 elements 全部の messages をフォワードして pipeline から取れるようにする
        // ちなみに messages と events の違い:
        // - messages はアプリケーションと elements の非同期メッセージ
        // - events は elements 間の非同期メッセージ
        // .message_forward(true)
        .name("main_pipeline")
        .build();

    let filesrc_el = match gst::ElementFactory::make("filesrc").name("src").property("location", path).build() {
        Ok(el) => el,
        Err(err) => {
            panic!("Failed to make filesrc element: {}", err);
        },
    };

    /*
    let qtdemux_el = match gst::ElementFactory::make("qtdemux").name("src").build() {
        Ok(el) => el,
        Err(err) => {
            panic!("Failed to make qtdemux element: {}", err);
        },
    };
    */

    let fakesink_el = match gst::ElementFactory::make("fakesink").name("sink").property("signal-handoffs", true).build() {
        Ok(el) => el,
        Err(err) => {
            panic!("Failed to make fakesink element: {}", err);
        },
    };
    let handoff_signal_handler_id = fakesink_el.connect("handoff", false, |args| {
        log::trace!("Started handling handoff signal: {:?}", thread::current().id());

        let src_el = args[0].get::<gst::Element>().expect("handoff signal must supply src element");
        assert_eq!(src_el.name(), "sink");

        let buffer = args[1].get::<gst::Buffer>().expect("handoff signal must supply buffer");

        /*
        let map = match buffer.map_readable() {
            Ok(map) => map,
            Err(err) => {
                panic!("Failed to map info: {}", err);
            },
        };
        */

        log::trace!("Received buffer: {:?}", buffer);

        // filesrc
        // BufferMap(Buffer { ptr: 0x7fc700f050c0, pts: --:--:--.---------, dts: 0:00:00.000000000, duration: --:--:--.---------, size: 4096, offset : 0, offset_end: 4096, flags: BufferFlags(DISCONT), metas: [] })

        None
    });
    log::debug!("Set handoff signal handler: {:?}", handoff_signal_handler_id);


    if let Err(err) = pipeline.add_many(&[&filesrc_el, &fakesink_el]) {
        panic!("Failed to add elements to pipeline: {}", err);
    };

    if let Err(err) = gst::Element::link_many(&[&filesrc_el, &fakesink_el]) {
        panic!("Failed to link elements: {}", err);
    };

    // 任意の pipeline が与えられて play するようなシチュエーションでかつ
    // EOS Message で終了とみなすなら。
    // children が 0 を除外しないといけない。
    // children=0 だと EOS はこないが pipeline は play されているとは言えないので
    assert_ne!(pipeline.children().len(), 0);

    // 接続忘れはバグなので assert で殺す
    assert_eq!((pipeline.find_unlinked_pad(gst::PadDirection::Sink), pipeline.find_unlinked_pad(gst::PadDirection::Src)), (None, None));

    // set_state したら、次のステップで状態が変わる時もあれば変わらない時もある
    // あくまでも target となる state を set して、状態の遷移は非同期に行われることがある
    // 内部的には以下のように複数の state がある
    // - current_state
    // - pending_state
    // - next_state
    // - target_state
    //
    // pipeline は実際には set された state に向かって以下のような遷移を行う
    // - NULL → READY:
    //   bus の flush が終わったり、必要なかったらすぐ READY になる
    // - READY → PAUSED:
    //   running_time を 0 にリセットにされた状態
    // - PAUSED → PLAYING:
    //   Clock の選択（変換の時は使わないけど、再生の時に開始位置と現在の時間から再生位置を合わせるために必要）
    //   PLAYING になった時の Clock time が base_time になる（base_time + running_time の位置が処理すべきフレームや音声になる）
    //   再生タスクの場合は latency もここで計算される
    //   全ての elements に base_time と clock が伝達される
    // - PLAYING → PAUSED:
    //   処理が止まっているけど base_time, running_time は保持している
    // - PAUSED → NULL:
    //   bus の flush が始まる。必要がないならすぐ READY になる
    //
    // メモ: Clock, running_time, latency
    // は再生の時に、どこまで再生してどのフレームを描画してどのペースでレンダリングすればいいかを調整するために使われる。変換タスクでは使われない。たぶん。
    //
    // 上記の説明はあくまでも内部的な話で、 get_state では内部的な state を知ることはできないっぽい。
    // message を通じて本当の内部 state を知ることができる

    assert_eq!(pipeline.state(None), (Ok(gst::StateChangeSuccess::Success), gst::State::Null, gst::State::VoidPending));

    match pipeline.set_state(gst::State::Playing) {
        Ok(gst::StateChangeSuccess::Success) => {
            // 成功 (
            log::debug!("Set pipeline playing immediately");
        },
        Ok(gst::StateChangeSuccess::Async) => {
            // 非同期処理で変更されるのでコールバックまち
            log::debug!("Started to set pipeline playing async");
        },
        Ok(gst::StateChangeSuccess::NoPreroll) => {
            // ライブ配信などで、 Paused にしても続きから再生できないとき
            // あくまでも Success なので、状態遷移は行われている
            // 今回は NULL -> PLAYING なので発生しない
            unreachable!();
        },
        Err(gst::StateChangeError) => {
            // state を変更できなかった。返り値にエラーメッセージは含まれない
            log::error!("Failed to set pileline playing");

            // bus から取得できないか試みる
            if let Some(bus) = pipeline.bus() {
                while let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error]) {
                    match msg.view() {
                        gst::message::MessageView::Error(err) => {
                            panic!("Error from {:?}: {} ({:?})", msg.src().map(|s| s.path_string()), err.error(), err.debug());
                        },
                        _ => unreachable!(),
                    }
                }
            }
        },
    }


    let bus = pipeline.bus().expect("The bus must exist when the pipeline exists. I don't know when it happens");

    // bus を監視する。メッセージがあれば iter_timed が中で msg を timed_pop してくる。
    // iter だと non-blocking メソッドとなりすぐに return しちゃう
    // これ自体が event loop なわけではなく、 msg queue を poll しているだけ
    for msg in bus.iter_timed(gst::ClockTime::NONE) {
        match msg.view() {
            gst::MessageView::StateChanged(state_changed) => {
                let el = state_changed.src()
                    .expect("State changed message must be sent from element")
                    .downcast_ref::<gst::Element>()
                    .expect("State changed message must be sent from element");

                let old_state = state_changed.old();
                let current_state = state_changed.current();
                let pending_state = state_changed.pending();

                log::debug!("MESSAGE: State changed: [{}] {:?} -> {:?} {}", el.name(), old_state, current_state, if pending_state == gst::State::VoidPending { "".into() } else { format!("(final: {:?})", pending_state) });
            },
            gst::MessageView::StreamStatus(stream_status) => {
                let pad = stream_status.src()
                    .expect("Stream status message must be sent from pad")
                    .downcast_ref::<gst::Pad>()
                    .expect("Stream status message must be sent from pad");

                let (status_type, owner_el) = stream_status.get();
                assert_eq!(pad.parent_element().unwrap().name(), owner_el.name());

                // 公式 doc 曰く
                // gst::Task が入っているが、将来にわたってそういう保証はないとのこと
                let task = stream_status.stream_status_object()
                    .expect("Stream status object must be given")
                    .get::<gst::Task>()
                    .ok();

                // stream に関連づけられた task の名前はそのストリームが最初に出てきた element_name:pad_name らしい。
                // 例えば安直なコードだと filesrc0:src とかになる

                // [el_name:pad_name] (task_state) stream_status
                log::debug!("MESSAGE: Stream: [{}:{}] (task={}) {:?}", owner_el.name(), pad.name(), task.map(|task| format!("{:?}", task.state())).unwrap_or("Unknown".into()), status_type);
            },
            gst::MessageView::AsyncDone(async_done) => {
                // AsyncDone メッセージは 全ての sink elements の preroll が済んだことを示している
                // ちなみにこれと対をなす AsyncStart は pipeline まで upward されずに pipeline
                // で消費される。なぜ、そうなっているのかはわからないけど、 gstbin.c の
                // gst_bin_handle_message_func 関数のヘッダコメントにそう書かれてる
                let pipeline = async_done.src()
                    .expect("Async done message must be sent from pipeline")
                    .downcast_ref::<gst::Pipeline>()
                    .expect("Async done message must be sent from pipeline");
                assert_eq!(pipeline.name(), "main_pipeline");

                log::debug!("MESSAGE: Async (Preroll) done: [{}] {}", pipeline.name(), if let Some(time) = async_done.running_time() { time.to_string() } else { "".into() });
            },
            gst::MessageView::StreamStart(stream_start) => {
                let pipeline = stream_start.src()
                    .expect("Stream start message must be sent from pipeline")
                    .downcast_ref::<gst::Pipeline>()
                    .expect("Stream start message must be sent from pipeline");
                assert_eq!(pipeline.name(), "main_pipeline");

                // stream は group_id というものを持つ、 stream の group_id は以下のような振る舞いをする
                // - demux, queue など元の stream を派生させたり透過させたりする場合、そのストリームは派生元の group_id を持つ
                // - filesrc, audiotestsrc など一からストリームを作るような stream は group_id を新しく割り当てる
                // - mux など複数のストリームから一つのストリームを作るような stream も group_id を新しく割り当てる
                //
                // この挙動によって、どのストリームの源泉から来たストリームなのかを確認することができる
                let group_id = stream_start.group_id().expect("Stream start must have group_id");

                log::debug!("MESSAGE: Stream started: [{}] {:?}", pipeline.name(), group_id);
            },
            gst::MessageView::Latency(latency) => {
                let el = latency.src()
                    .expect("Latency message must be sent from element")
                    .downcast_ref::<gst::Element>()
                    .expect("Latency message must be sent from element");

                log::debug!("MESSAGE: Need recalculate latency: [{}]", el.name());
                if let Err(err) = pipeline.recalculate_latency() {
                    panic!("Failed to recalculate latency when receiving LATENCY message: {}", err);
                };
            },
            gst::MessageView::NewClock(new_clock) => {
                let pipeline = new_clock.src()
                    .expect("New clock message must be sent from pipeline")
                    .downcast_ref::<gst::Pipeline>()
                    .expect("New clock message must be sent from pipeline");
                assert_eq!(pipeline.name(), "main_pipeline");

                let clock = new_clock.clock().expect("New clock message must have a new clock object");

                log::debug!("MESSAGE: Clock set: [{}] {:?}", pipeline.name(), clock);
            },
            gst::MessageView::Eos(eos) => {
                let pipeline = eos.src()
                    .expect("EOS message must be sent from pipeline")
                    .downcast_ref::<gst::Pipeline>()
                    .expect("EOS message must be sent from pipeline");
                assert_eq!(pipeline.name(), "main_pipeline");

                log::debug!("MESSAGE: EOS: [{}]", pipeline.name());
                break;
            },
            gst::MessageView::Error(err) => {
                panic!("Error from {:?}: {} ({:?})", msg.src().map(|s| s.path_string()), err.error(), err.debug());
            }
            view => {
                log::info!("MESSAGE: Unknown message: {:?}", view);
            },
        }
    }

    assert_eq!(pipeline.state(None), (Ok(gst::StateChangeSuccess::Success), gst::State::Playing, gst::State::VoidPending));

    match pipeline.set_state(gst::State::Null) {
        Ok(gst::StateChangeSuccess::Success) => {
            // 成功
            log::debug!("Set pipeline null immediately");
        },
        Ok(gst::StateChangeSuccess::Async) => {
            // 非同期処理で変更された場合
            log::debug!("Started to set pipeline null async");
        },
        Ok(gst::StateChangeSuccess::NoPreroll) => {
            // XXX ここにくるパターンを体験してない
            log::debug!("Set pipeline null immediately, and knowing the stream coudln't be prerolled: {:?}", pipeline.state(None));
        },
        Err(gst::StateChangeError) => {
            // state を変更できなかった。返り値にエラーメッセージは含まれない
            log::error!("Failed to set pileline null");

            // bus から取得できないか試みる
            if let Some(bus) = pipeline.bus() {
                while let Some(msg) = bus.pop_filtered(&[gst::MessageType::Error]) {
                    match msg.view() {
                        gst::message::MessageView::Error(err) => {
                            panic!("Error from {:?}: {} ({:?})", msg.src().map(|s| s.path_string()), err.error(), err.debug());
                        },
                        _ => unreachable!(),
                    }
                }
            }
        },
    }

    assert_eq!(pipeline.state(None), (Ok(gst::StateChangeSuccess::Success), gst::State::Null, gst::State::VoidPending));

    for msg in bus.iter() {
        log::debug!("MESSAGE: Remaining message after EOS: {:?}", msg.view());
    }
}

