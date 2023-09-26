use std::{env, path::Path};

use gstreamer as gst;
use gst::prelude::*;

use log;
use env_logger;

fn main() {
    env_logger::init();

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

    let fakesink_el = match gst::ElementFactory::make("fakesink").name("sink").build() {
        Ok(el) => el,
        Err(err) => {
            panic!("Failed to make fakesink element: {}", err);
        },
    };

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
    // 以下のように複数の state がある
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

    assert_eq!(pipeline.state(None), (Ok(gst::StateChangeSuccess::Success), gst::State::Null, gst::State::VoidPending));

    match pipeline.set_state(gst::State::Playing) {
        Ok(gst::StateChangeSuccess::Success) => {
            // 成功
            assert_eq!(pipeline.state(None), (Ok(gst::StateChangeSuccess::Success), gst::State::Playing, gst::State::VoidPending));
            log::debug!("Set pipeline playing immediately");
        },
        Ok(gst::StateChangeSuccess::Async) => {
            // 非同期処理で変更されるのでコールバックまち
            // XXX ここにくるパターンを体験してない
            // state がどのような状態になっているかを見て、何か assert でかけることがあったらここに書く
            log::debug!("Started to set pipeline playing async: {:?}", pipeline.state(None));
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
            gstreamer::MessageView::Eos(eos) => {
                // XXX I don't know if eos has usable info or not
                log::debug!("Ended pipeline: {:?}", eos);
                break;
            },
            gstreamer::MessageView::Error(err) => {
                panic!("Error from {:?}: {} ({:?})", msg.src().map(|s| s.path_string()), err.error(), err.debug());
            }
            view => {
                log::debug!("Unknown message: {:?}", view);
            },
        }
    }

    assert_eq!(pipeline.state(None), (Ok(gst::StateChangeSuccess::Success), gst::State::Playing, gst::State::VoidPending));

    match pipeline.set_state(gstreamer::State::Null) {
        Ok(gst::StateChangeSuccess::Success) => {
            // 成功
            assert_eq!(pipeline.state(None), (Ok(gst::StateChangeSuccess::Success), gst::State::Null, gst::State::VoidPending));
            log::debug!("Set pipeline null immediately");
        },
        Ok(gst::StateChangeSuccess::Async) => {
            // 非同期処理で変更されるのでコールバックまち
            // XXX ここにくるパターンを体験してない
            // state がどのような状態になっているかを見て、何か assert でかけることがあったらここに書く
            log::debug!("Started to set pipeline null async: {:?}", pipeline.state(None));
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

    for msg in bus.iter() {
        log::debug!("Remaining message after EOS: {:?}", msg.view());
    }
}

