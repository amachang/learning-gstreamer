use std::{env, process};

use gstreamer as gst;
use gst::prelude::*;

use log;
use env_logger;

fn main() {
    env_logger::init();

    let args = env::args().collect::<Vec<_>>();
    if args.len() != 2 {
        eprintln!("Usage: {} <h264_isomp4_file_path>", args[0]);
    }

    if let Err(err) = gst::init() {
        log::error!("Failed to init gstreamer: {}", err);
        process::exit(1);
    }

    let pipeline = gst::Pipeline::builder()
        // 含まれる子 elements 全部の messages をフォワードして pipeline から取れるようにする
        // ちなみに messages と events の違い:
        // - messages はアプリケーションと elements の非同期メッセージ
        // - events は elements 間の非同期メッセージ
        .message_forward(true)
        .name("main_pipeline")
        .build();

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
            assert_eq!(pipeline.state(None), (Ok(gst::StateChangeSuccess::Async), gst::State::Ready, gst::State::Playing));
            log::debug!("Started to set pipeline playing async");
        },
        Ok(gst::StateChangeSuccess::NoPreroll) => {
            // ライブ配信などで、 Paused にしても続きから再生できないとき
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
                            log::error!("Error from {:?}: {} ({:?})", msg.src().map(|s| s.path_string()), err.error(), err.debug());
                            process::exit(1);
                        },
                        _ => unreachable!(),
                    }
                }
            }
        },
    }

}

