//! Strict second-precision RFC3339 deadlines for temporary LFS action URLs.
use std::time::{Duration,Instant,SystemTime,UNIX_EPOCH};
type Result<T>=std::result::Result<T,String>;
fn timestamp(value:&str)->Result<SystemTime>{
 let invalid=||"INVALID_LFS_ACTION_EXPIRY".to_string();
 if !value.is_ascii()||!matches!(value.len(),20|25){return Err(invalid());}
 let bytes=value.as_bytes();
 if bytes[4]!=b'-'||bytes[7]!=b'-'||bytes[10]!=b'T'||bytes[13]!=b':'||bytes[16]!=b':'{return Err(invalid());}
 let number=|start:usize,end:usize|->Result<i64>{let part=&value[start..end];if !part.bytes().all(|b|b.is_ascii_digit()){return Err(invalid());}part.parse().map_err(|_|invalid())};
 let(year,month,day,hour,minute,second)=(number(0,4)?,number(5,7)?,number(8,10)?,number(11,13)?,number(14,16)?,number(17,19)?);
 if year<1970||!(1..=12).contains(&month)||hour>23||minute>59||second>59{return Err(invalid());}
 let leap=year%4==0&&(year%100!=0||year%400==0);
 let lengths=[31,if leap{29}else{28},31,30,31,30,31,31,30,31,30,31];
 if day<1||day>lengths[(month-1)as usize]{return Err(invalid());}
 let before_year=|year:i64|{let previous=year-1;365*previous+previous/4-previous/100+previous/400};
 let days=before_year(year)-before_year(1970)+lengths[..(month-1)as usize].iter().sum::<i64>()+day-1;
 let offset=if value.len()==20{if bytes[19]!=b'Z'{return Err(invalid());}0}else{
  if !matches!(bytes[19],b'+'|b'-')||bytes[22]!=b':'{return Err(invalid());}
  let(h,m)=(number(20,22)?,number(23,25)?);if h>23||m>59{return Err(invalid());}
  (h*3600+m*60)*if bytes[19]==b'+'{1}else{-1}
 };
 let seconds=days*86400+hour*3600+minute*60+second-offset;
 if seconds<0{return Err(invalid());}
 UNIX_EPOCH.checked_add(Duration::from_secs(seconds as u64)).ok_or_else(invalid)
}
pub(super) fn deadline(relative:Option<i32>,absolute:Option<&str>)->Result<Option<Instant>>{
 let duration=if let Some(seconds)=relative{
  if seconds<=0{return Err("LFS_ACTION_EXPIRED".into());}Some(Duration::from_secs(seconds as u64))
 }else if let Some(value)=absolute{
  Some(timestamp(value)?.duration_since(SystemTime::now()).map_err(|_|"LFS_ACTION_EXPIRED")?)
 }else{None};
 duration.map(|duration|{if duration.is_zero(){return Err("LFS_ACTION_EXPIRED".into());}Instant::now().checked_add(duration).ok_or_else(||"INVALID_LFS_ACTION_EXPIRY".into())}).transpose()
}
