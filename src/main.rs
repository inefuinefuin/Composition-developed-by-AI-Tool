// src/main.rs
use std::env;
use std::fs::File;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::thread;
use std::collections::VecDeque;

use symphonia::core::audio::SampleBuffer;
use symphonia::core::codecs::{DecoderOptions, CODEC_TYPE_NULL};
use symphonia::core::errors::Error;
use symphonia::core::formats::FormatOptions;
use symphonia::core::io::MediaSourceStream;
use symphonia::core::meta::MetadataOptions;
use symphonia::core::probe::Hint;
use symphonia::core::units::Time;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use single_instance::SingleInstance;
use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};

struct AudioPlayer {
    file_path: String,
    is_paused: Arc<Mutex<bool>>,
    should_stop: Arc<Mutex<bool>>,
    seek_position: Arc<Mutex<Option<f64>>>,
    volume: Arc<Mutex<f32>>,
    current_time: Arc<Mutex<f64>>,  // 当前播放位置（秒）
}

impl AudioPlayer {
    fn new(file_path: String) -> Self {
        Self {
            file_path,
            is_paused: Arc::new(Mutex::new(false)),
            should_stop: Arc::new(Mutex::new(false)),
            seek_position: Arc::new(Mutex::new(None)),
            volume: Arc::new(Mutex::new(1.0)),
            current_time: Arc::new(Mutex::new(0.0)),
        }
    }

    fn play(&mut self) -> Result<(), Box<dyn std::error::Error>> {
        let file = File::open(&self.file_path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let mut hint = Hint::new();
        if let Some(ext) = std::path::Path::new(&self.file_path).extension() {
            hint.with_extension(ext.to_str().unwrap());
        }

        let meta_opts = MetadataOptions::default();
        let fmt_opts = FormatOptions::default();

        let probed = symphonia::default::get_probe().format(&hint, mss, &fmt_opts, &meta_opts)?;
        let mut format = probed.format;

        let track = format
            .tracks()
            .iter()
            .find(|t| t.codec_params.codec != CODEC_TYPE_NULL)
            .ok_or("找不到音頻軌道")?;

        let track_id = track.id;
        let mut decoder = symphonia::default::get_codecs()
            .make(&track.codec_params, &DecoderOptions::default())?;

        let input_sample_rate = *track.codec_params.sample_rate.as_ref().ok_or("無法獲取採樣率")?;
        let input_channels = track.codec_params.channels.as_ref().ok_or("無法獲取聲道信息")?.count();

        // 初始化 CPAL 音頻輸出
        let host = cpal::default_host();
        let device = host.default_output_device().ok_or("找不到輸出設備")?;
        let config = device.default_output_config()?;
        
        let output_sample_rate = config.sample_rate().0;
        let output_channels = config.channels() as usize;
        
        println!("\n輸入: {}Hz, {} 聲道", input_sample_rate, input_channels);
        println!("輸出: {}Hz, {} 聲道\n", output_sample_rate, output_channels);

        let sample_buffer: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::with_capacity(48000 * 2)));
        
        // 为闭包克隆引用
        let is_paused_clone = Arc::clone(&self.is_paused);
        let volume_clone = Arc::clone(&self.volume);
        let sample_buffer_clone = Arc::clone(&sample_buffer);

        // 創建音頻流
        let stream = device.build_output_stream(
            &config.config(),
            move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                let paused = *is_paused_clone.lock().unwrap();
                let vol = *volume_clone.lock().unwrap();
                
                if paused {
                    for sample in data.iter_mut() {
                        *sample = 0.0;
                    }
                    return;
                }

                let mut buffer = sample_buffer_clone.lock().unwrap();
                for sample in data.iter_mut() {
                    *sample = buffer.pop_front().unwrap_or(0.0) * vol;
                }
            },
            |err| eprintln!("音頻流錯誤: {}", err),
            None,
        )?;

        stream.play()?;

        // 解碼循環
        loop {
            if *self.should_stop.lock().unwrap() {
                break;
            }

            // 檢查是否正在暫停，暫停時不解碼
            if *self.is_paused.lock().unwrap() {
                std::thread::sleep(Duration::from_millis(10));
                continue;
            }

            // 檢查是否需要跳轉
            if let Some(pos) = self.seek_position.lock().unwrap().take() {
                let time = Time::from(pos);
                if let Err(e) = format.seek(
                    symphonia::core::formats::SeekMode::Accurate,
                    symphonia::core::formats::SeekTo::Time { time, track_id: Some(track_id) },
                ) {
                    eprintln!("跳轉失敗: {}", e);
                } else {
                    // 更新当前播放位置
                    *self.current_time.lock().unwrap() = pos;
                }
            }

            let packet = match format.next_packet() {
                Ok(packet) => packet,
                Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
                Err(e) => {
                    eprintln!("讀取包錯誤: {}", e);
                    break;
                }
            };

            if packet.track_id() != track_id {
                continue;
            }

            match decoder.decode(&packet) {
                Ok(decoded) => {
                    let mut buf = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
                    buf.copy_interleaved_ref(decoded);
                    
                    // 检查缓冲区并等待消费
                    loop {
                        let sample_buf = sample_buffer.lock().unwrap();
                        
                        if sample_buf.len() <= output_sample_rate as usize * output_channels * 2 {
                            // 缓冲区不大，可以添加数据
                            break;
                        }
                        
                        // 缓冲区太大，释放锁并等待
                        drop(sample_buf);
                        std::thread::sleep(Duration::from_millis(5));
                        
                        // 检查是否需要停止
                        if *self.should_stop.lock().unwrap() {
                            return Ok(());
                        }
                        
                        // 检查是否跳转，如果是则清空缓冲区
                        if self.seek_position.lock().unwrap().is_some() {
                            let mut buf = sample_buffer.lock().unwrap();
                            buf.clear();
                            break;
                        }
                    }
                    
                    // 重新获取锁以添加样本
                    let mut sample_buf = sample_buffer.lock().unwrap();
                    let samples = buf.samples();
                    
                    // 计算此包的时长并更新当前时间
                    let duration_seconds = samples.len() as f64 / (input_sample_rate as f64 * input_channels as f64);
                    *self.current_time.lock().unwrap() += duration_seconds;
                    
                    // 声道转换
                    if input_channels == output_channels {
                        // 声道数相同，直接复制
                        sample_buf.extend(samples.iter());
                    } else {
                        let frame_count = samples.len() / input_channels;
                        
                        for i in 0..frame_count {
                            let frame_start = i * input_channels;
                            
                            match (input_channels, output_channels) {
                                (1, 2) => {
                                    // 单声道 -> 立体声
                                    let mono = samples[frame_start];
                                    sample_buf.push_back(mono);
                                    sample_buf.push_back(mono);
                                }
                                (2, 1) => {
                                    // 立体声 -> 单声道
                                    sample_buf.push_back((samples[frame_start] + samples[frame_start + 1]) / 2.0);
                                }
                                _ if input_channels >= output_channels => {
                                    // 多声道 -> 少声道
                                    for ch in 0..output_channels {
                                        sample_buf.push_back(samples[frame_start + ch]);
                                    }
                                }
                                _ => {
                                    // 少声道 -> 多声道
                                    for _ in 0..output_channels {
                                        sample_buf.push_back(samples[frame_start]);
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => eprintln!("解碼錯誤: {}", e),
            }
        }

        drop(stream);
        Ok(())
    }
}

fn main() {
    // 使用鎖機制確保單一實例運行
    let instance = SingleInstance::new("musicPlayer-unique-instance").unwrap();
    if !instance.is_single() {
        eprintln!("程序已經在運行中！");
        std::process::exit(1);
    }

    // 獲取命令行參數
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: cargo run -- <audio_file_path>");
        std::process::exit(1);
    }

    let file_path = args[1].clone();

    // 創建播放器
    let mut player = AudioPlayer::new(file_path);
    let player_paused = Arc::clone(&player.is_paused);
    let player_stop = Arc::clone(&player.should_stop);
    let player_seek = Arc::clone(&player.seek_position);
    let player_volume = Arc::clone(&player.volume);
    let player_time = Arc::clone(&player.current_time);

    // 在新線程中播放
    let play_thread = thread::spawn(move || {
        if let Err(e) = player.play() {
            eprintln!("播放錯誤: {}", e);
        }
    });

    // 啟用終端原始模式
    enable_raw_mode().unwrap();

    println!("=========================================");
    println!("音樂播放器控制：");
    println!("  [空格] - 暫停/繼續");
    println!("  [←] - 後退 5 秒");
    println!("  [→] - 前進 5 秒");
    println!("  [↑] - 音量增加");
    println!("  [↓] - 音量減少");
    println!("  [q] - 退出");
    println!("=========================================\n");

    // 主控制循環
    loop {
        // 检查播放线程是否已结束
        if play_thread.is_finished() {
            println!("\n播放完成！");
            break;
        }
        
        if *player_stop.lock().unwrap() {
            break;
        }

        // 非阻塞地讀取按鍵事件
        if event::poll(Duration::from_millis(100)).unwrap() {
            if let Event::Key(KeyEvent { code, kind, .. }) = event::read().unwrap() {
                // 只处理按键按下事件，忽略释放事件
                if kind != KeyEventKind::Press {
                    continue;
                }
                
                match code {
                    KeyCode::Char(' ') => {
                        let mut paused = player_paused.lock().unwrap();
                        *paused = !*paused;
                        if *paused {
                            println!("⏸ 已暫停");
                        } else {
                            println!("▶ 繼續播放");
                        }
                    }
                    KeyCode::Left => {
                        let current = *player_time.lock().unwrap();
                        let new_position = (current - 5.0).max(0.0);
                        let mut seek = player_seek.lock().unwrap();
                        *seek = Some(new_position);
                        println!("⏪ 後退 5 秒 (位置: {:.1}s)", new_position);
                    }
                    KeyCode::Right => {
                        let current = *player_time.lock().unwrap();
                        let new_position = current + 5.0;
                        let mut seek = player_seek.lock().unwrap();
                        *seek = Some(new_position);
                        println!("⏩ 前進 5 秒 (位置: {:.1}s)", new_position);
                    }
                    KeyCode::Up => {
                        let mut vol = player_volume.lock().unwrap();
                        *vol = (*vol + 0.1).min(2.0);
                        println!("🔊 音量: {:.0}%", *vol * 100.0);
                    }
                    KeyCode::Down => {
                        let mut vol = player_volume.lock().unwrap();
                        *vol = (*vol - 0.1).max(0.0);
                        println!("🔉 音量: {:.0}%", *vol * 100.0);
                    }
                    KeyCode::Char('q') | KeyCode::Char('Q') => {
                        println!("\n退出播放器...");
                        let mut stop = player_stop.lock().unwrap();
                        *stop = true;
                        break;
                    }
                    _ => {}
                }
            }
        }
    }

    // 等待播放線程結束
    play_thread.join().unwrap();

    // 恢復終端模式
    disable_raw_mode().unwrap();
    println!("播放器已關閉。");
}
