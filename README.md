# GStreamer ちゃんとやる

- Gstreamer だと ffmpeg 系のライブラリより細かいことができる
  - video file や stream と player の間に流れる yuv とかを分析
  - 壊れた h264 の nal を分析したり、
- 個人的に機械学習とかで、動画の音声の波形とか、フレームとかを、 UI やファイル出力に繋ぐ中間のレイヤーで何かしたい
- ぱぱっとやると使えるのだが、急に queue が full になったり、 caps 間の negotiation がうまくいかなかったり、カジュアルに学んだだけではちゃんとした品質で何かの価値を生み出すことはできないと悟った

[GStreamer の勉強メモ](markdown/what_i_know.md)
  
