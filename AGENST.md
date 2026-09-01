# Todos

[•] Design novo CachedWave (lods min/max por coluna, chunk=samples/width, sobras no último) + WaveformCache LRU em waveform.rs; build_lods; nivel_por_largura; bake gain+fades; to_clip_data com mins/maxes
[ ] waveform.rs: reescrever wire_resolution_requests sem raster (só publicar lods do nível certo), reproject_clip, split/merge/trim_por_lods
[ ] decode_worker.rs: remover Raster/apply_gain_and_fade/imagem; DecodedWave/ImportedWave com lods (app type); import_wave build_lods; schedule_import
[ ] clip_cache.rs: CachedSample.wave com lods (build_lods); remover peaks/imagem/mono/mip
[ ] importer.rs: import_audio_file/async/from_entry/untrimmed com lods; beat.rs schedule_decode sem with_image
[ ] clip_data.slint: trocar waveform-points/image/src-x/w por waveform-mins/waveform-maxes
[ ] audio_waveform.slint: desenhar barras min→max por coluna (fallback peaks p/ sampler)
[ ] track_clip.slint: remover imagem, usar waveform-mins/maxes, guards, ClipDragState mins/maxes
[ ] track_row.slint, clip_drag_state.slint, clip_drag_overlay.slint: wire mins/maxes
[ ] timeline.rs:294/reproject/wire_resolution call sites; main.rs WaveformCache::new(cap); cargo check + tests
