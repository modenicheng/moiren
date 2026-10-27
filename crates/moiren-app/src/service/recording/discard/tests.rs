use super::*;

#[test]
fn discard_writer_supports_wav_header_seeks_without_buffering_samples() {
    let mut writer = DiscardWriter::default();
    assert_eq!(writer.seek(SeekFrom::End(0)).unwrap(), 0);
    writer.write_all(&[1; 58]).unwrap();
    writer.write_all(&[2; 4096]).unwrap();
    writer.seek(SeekFrom::Start(4)).unwrap();
    writer.write_all(&[0; 4]).unwrap();
    assert_eq!(writer.seek(SeekFrom::End(0)).unwrap(), 4154);
    assert_eq!(writer.seek(SeekFrom::Current(-4)).unwrap(), 4150);
    assert!(writer.seek(SeekFrom::Start(0)).is_ok());
    assert!(writer.seek(SeekFrom::Current(-1)).is_err());
    assert_eq!(writer.length, 4154);
}
