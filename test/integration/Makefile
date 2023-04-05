.PHONY: all get_input_files test webm_test update_input_check update_output_check clean

HISUI=../../release/hisui --openh264 /usr/local/lib/libopenh264-2.1.1-linux64.6.so
LIBFKD_AAC_DEV_VERSION=$(shell dpkg -s libfdk-aac-dev | grep Version | cut -d ' ' -f 2)

all: test

get_input_files:
	[ -f input/caminandes3.webm ] || curl -o input/caminandes3.webm https://ia801606.us.archive.org/24/items/CaminandesLlamigos/Caminandes_%20Llamigos-1080p.webm # CC BY 3.0 https://creativecommons.org/licenses/by/3.0/ (c) copyright 2016, Blender Foundation / http://www.caminandes.com/
	[ -f input/lilac_blossom_bloom_spring.jpg ] || curl -o input/lilac_blossom_bloom_spring.jpg https://free-images.com/tn/c1df/lilac_blossom_bloom_spring.jpg # CC0
	[ -f input/Big_Buck_Bunny_360_10s_1MB.webm ] || curl -o input/Big_Buck_Bunny_360_10s_1MB.webm https://test-videos.co.uk/vids/bigbuckbunny/webm/vp9/360/Big_Buck_Bunny_360_10s_1MB.webm # CC BY 3.0 https://creativecommons.org/licenses/by/3.0/ (c) copyright 2008, Blender Foundation / www.bigbuckbunny.org
	sha224sum -c input/check

test: webm_test mp4_test layout_webm_test layout_mp4_test
	[ -f "output/check_libfdk-aac-dev-$(LIBFKD_AAC_DEV_VERSION)" ] && sha224sum -c output/check_libfdk-aac-dev-$(LIBFKD_AAC_DEV_VERSION)

#         case3, case4 の vp8 の場合に, 結果が不定となる場合があった
#         case4 の場合を調査したところ, 同じ hisui のバイナリから生成した WebM のクラスター部で, 映像の最初の Intraframe の次の Interframe の 4バイト目が 0x0b だったり 0x0d だったりしている
webm_test: get_input_files
	rm -f output/*.webm*
	for m in metadata/*.json; do \
		base=$$(basename $${m} .json); \
		echo webm_$${base}; \
		vp9_file=output/$${base}.vp9.webm; \
		vp8_file=output/$${base}.vp8.webm; \
		audio_file=output/$${base}.weba; \
		${HISUI} -f $${m} --out-file $${vp9_file} --show-progress-bar false --log-level error --out-video-codec VP9; \
		${HISUI} -f $${m} --out-file $${audio_file} --show-progress-bar false --log-level error --audio-only; \
		tail --bytes=+$$(bash ./get_cluster_start_position.bash $${vp9_file}) $${vp9_file} | head --bytes=$$(bash ./get_cluster_size.bash $${vp9_file}) > $${vp9_file}.cluster; \
		tail --bytes=+$$(bash ./get_cluster_start_position.bash $${audio_file}) $${audio_file} | head --bytes=$$(bash ./get_cluster_size.bash $${audio_file}) > $${audio_file}.cluster; \
		[ x"$$base" = x"case3" -o x"$$base" = x"case4" ] || ${HISUI} -f $${m} --out-file $${vp8_file} --show-progress-bar false --log-level error --out-video-codec VP8; \
		[ x"$$base" = x"case3" -o x"$$base" = x"case4" ] || tail --bytes=+$$(bash ./get_cluster_start_position.bash $${vp8_file}) $${vp8_file} | head --bytes=$$(bash ./get_cluster_size.bash $${vp8_file}) > $${vp8_file}.cluster; \
	done

mp4_test: get_input_files
	rm -f output/*.mp4
	for m in metadata/*.json; do \
		base=$$(basename $${m} .json); \
		echo mp4_$${base}; \
		opus_file=output/$${base}.opus.mp4; \
		simple_opus_file=output/$${base}.simple.opus.mp4; \
		aac_file=output/$${base}.aac.mp4; \
		opus_m4a_file=output/$${base}.opus.m4a; \
		aac_m4a_file=output/$${base}.aac.m4a; \
		faketime -f '2021-01-06 00:00:00' ${HISUI} -f $${m} --out-file $${opus_file} --show-progress-bar false --log-level error --out-container mp4; \
		faketime -f '2021-01-06 00:00:00' ${HISUI} -f $${m} --out-file $${simple_opus_file} --show-progress-bar false --log-level error --out-container mp4 --mp4-muxer simple; \
		faketime -f '2021-01-06 00:00:00' ${HISUI} -f $${m} --out-file $${aac_file} --show-progress-bar false --log-level error --out-container mp4 --out-audio-codec aac; \
		faketime -f '2021-01-06 00:00:00' ${HISUI} -f $${m} --out-file $${opus_m4a_file} --show-progress-bar false --log-level error --out-container mp4 --audio-only; \
		faketime -f '2021-01-06 00:00:00' ${HISUI} -f $${m} --out-file $${aac_m4a_file} --show-progress-bar false --log-level error --out-container mp4 --out-audio-codec aac --audio-only; \
	done

layout_webm_test: get_input_files
	rm -f output/layout/case*.webm*
	for m in layout/webm/case*.json; do \
		base=$$(basename $${m} .json); \
		echo layout_webm_$${base}; \
		output=output/layout/$${base}.webm; \
		${HISUI} --layout $${m} --out-file $${output} --show-progress-bar false --log-level error --out-video-codec VP9; \
		tail --bytes=+$$(bash ./get_cluster_start_position.bash $${output}) $${output} | head --bytes=$$(bash ./get_cluster_size.bash $${output}) > $${output}.cluster; \
	done

layout_mp4_test: get_input_files
	rm -f output/layout/case*.mp4
	for m in layout/mp4/case*.json; do \
		base=$$(basename $${m} .json); \
		echo layout_mp4_$${base}; \
		output=output/layout/$${base}.mp4; \
		faketime -f '2021-12-23 00:00:00' ${HISUI} --layout $${m} --out-file $${output} --show-progress-bar false --log-level error --out-video-codec VP9; \
	done

update_input_check:
	sha224sum input/*.webm input/*.jpg > input/check

update_output_check:
	sha224sum output/*.cluster output/*.mp4 output/*.m4a output/layout/*.cluster output/layout/*.mp4 > output/check_libfdk-aac-dev-$(LIBFKD_AAC_DEV_VERSION)

clean:
	rm -f output/*.cluster output/*.mp4 output/*.webm output/layout/*.cluster output/layout/*.webm
