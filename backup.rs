// let mut writer = self.stream.as_mut().unwrap().put_with_stream(
//     filename
//         .to_str()
//         .ok_or(format!("Failed converting Path to str: {filename:?}"))
//         .map_err(FtpError::SecureError)?,
// )?;

// let mut buf = [0; 8 * 1024];
// let mut len = 0;

// loop {
//     match reader.read(&mut buf) {
//         Ok(0) => {
//             break;
//         }
//         Ok(n) => {
//             len += n;
//             writer.write_all(&buf[..n])?;
//             writer.flush()?;
//         }
//         Err(e) if e.kind() == ErrorKind::Interrupted => continue,
//         Err(e) => return Err(e.into()),
//     };
// }

// self.stream.as_mut().unwrap().finalize_put_stream(writer)?;
// Ok(len as u64)
