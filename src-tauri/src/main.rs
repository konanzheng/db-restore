// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// Learn more about Tauri commands at https://tauri.app/v1/guides/features/command
#[macro_use]
extern crate simple_log;
extern crate tar;
extern crate flate2;

use sqlx::{Executor, MySqlPool, mysql,Row,ConnectOptions};
use sqlx::mysql::MySqlConnectOptions;
use sqlx::mysql::MySqlSslMode;
use std::str::FromStr;
use std::time::Instant;
use std::time::Duration;
use std::env;
use std::path::Path;
use std::fs;
use std::{
    io::{BufRead, Write,BufReader,BufWriter},
    fs::{File},
};
use std::path::PathBuf;
use simple_log::LogConfig;
use serde_json::Value;

use std::io::prelude::*;
use tar::Archive;
use flate2::read::GzDecoder;

#[derive( Clone)]
struct Table{
    name: String,
    ddl: String,
    fields: String,
    data_files: Vec<PathBuf>,
    rows: u64,
}
fn main() {
    
    let config = r#"
    {
        "path":"./log/db-restore.log",
        "level":"info",
        "size":10,
        "out_kind":["console","file"],
        "roll_count":10,
        "time_format":"%H:%M:%S.%f"
    }"#;
    let mut log_config: LogConfig = serde_json::from_str(config).unwrap();
    log_config.path = env::temp_dir().join("log/db-restore.log").to_str().unwrap().to_string();
    simple_log::new(log_config).unwrap();//init log

    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![restore])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}



#[tauri::command]
async fn restore(file_path:&str,url:&str) ->Result<String, ()>{
    print!("开始导入文件:{},使用连接字符串{}",&file_path,url);
    const SIZE:u32 = 100;
    let  mut tables: Vec<Table> = Vec::<Table>::new();
    let  mut files: Vec<String> = Vec::<String>::new();
    let mut result = String::new();
    // 解压nb3文件
    if file_path.ends_with(".nb3") {
        tables = extract(file_path.clone());
        if tables.len() > 0 {
            println!("解析文件成功！,需要导入的表有{}个",tables.len());
        } else {
            result.push_str("解析文件失败！");
            info!("{}",result);
            return Ok(result);
        }
    } else {
        files = split(file_path.clone());
        if files.len() > 0 {
            info!("解析文件成功！,需要导入的sql有{}个",files.len());
        } else {
            result.push_str("解析文件失败！");
            info!("{}",result);
            return Ok(result);
        }
    }
    // let mut url = String::from(db_url);
    let pool :MySqlPool ;
    let mut opts = MySqlConnectOptions::from_str(url).unwrap();
    opts = opts.ssl_mode(MySqlSslMode::Disabled);
    opts.disable_statement_logging();
    let c =  mysql::MySqlPoolOptions::new().min_connections(SIZE).max_connections(SIZE).connect_timeout(Duration::from_secs(60)).idle_timeout(Duration::from_secs(30)).after_connect(|conn| Box::pin(async move {
        conn.execute("SET sql_log_bin=OFF;").await?;
        conn.execute("SET FOREIGN_KEY_CHECKS=0;").await?;
        conn.execute("SET global max_allowed_packet = 2*1024*1024*10;").await?;
 
        Ok(())
     }))
    .connect_with(opts).await;
    if c.is_err() {
        result = format!("错误!数据库连接字符串有误，请重新输入,详细错误信息:{:?}",c.err());
        info!("{}",result);
        Err(())
    } else {
        pool = c.unwrap();
        info!("数据库连接成功: {} 开始处理", url);
        let now = Instant::now();
        const SIZE:u32 = 100;
        info!("数据库连接池 : \n{:?}", pool);
        let fkc:u64 = sqlx::query("SELECT  @@FOREIGN_KEY_CHECKS as fkc;").fetch_one(&pool).await.unwrap().get_unchecked("fkc");
        info!("FOREIGN_KEY_CHECKS:{}",fkc);
        // 循环每个table处理
        let mut handles2 = Vec::with_capacity(10);
        for table in tables {
            handles2.push(tauri::async_runtime::spawn(execute(table.clone(),pool.clone())));
        }
        for handle in handles2 {
            let _r = handle.await;
        }
        let mut i = 0;
        // while i < tables.len() {
        //     let mut handles2 = Vec::with_capacity(10);
        //     while handles2.len()<10 && i < tables.len() {
        //         handles2.push(tauri::async_runtime::spawn(execute(tables[i].clone(),pool.clone())));
        //         i+=1;
        //     }
        //     for handle in handles2 {
        //         let _r = handle.await;
        //     }
        // }
        while i < files.len() {
            let mut j = 0;
            let mut handles2 = Vec::with_capacity(100);
            while j < 100 && i < files.len() {
                handles2.push(tauri::async_runtime::spawn(source(files[i].clone(),pool.clone())));
                i+=1;
                j+=1;
            }
            for handle in handles2 {
                let _r = handle.await;
            }
        }
        pool.close().await;
        let duration = now.elapsed().as_millis();    
        result = format!("导入成功,总耗时：{} ms", duration);
        info!("{}", result);        
        Ok(result)
    }
    // return result;
}
async fn insert(insert: String,pool2: MySqlPool) {
    let _r = pool2.execute(sqlx::query(&insert)).await;
    match _r {
        Ok(_ok) => {
        },
        Err(_e) => {
            info!("sql出错:{:?} ",_e);
        }
    }
}
async fn source(file: String, pool2: MySqlPool){
    println!("开始导入文件:{}",&file);
    let f = File::open(&file.clone()).unwrap();
    let f_len = f.metadata().unwrap().len();
    let mut reader = BufReader::new(f);
    let mut split_content = String::from("");
    // TODO 先执行关闭bin日志，关闭事务，关闭主键检查 操作再开始执行sql,批量插入数据条数调优每次处理10000条数据
    if f_len > 1024*1024*10 {
        let batch_insert_szie = 500;
        let mut ddl = String::from("");
        let mut insert_sqls = Vec::with_capacity(batch_insert_szie);
        loop {
            let mut insert_sql=String::from("");
            reader.read_line(&mut insert_sql).unwrap();
            let after = reader.stream_position().unwrap();
            let trim = insert_sql.trim();
            if trim.starts_with("INSERT") && trim.ends_with(";") {
                if ddl.len() > 0 {
                    let _r =pool2.execute(sqlx::query(&ddl)).await;
                    // println!("插入数据之前执行sql内容\n:{} ",ddl);
                }
                let _ =&insert_sqls.push(insert_sql);
            } else {
                ddl = ddl + insert_sql.trim();
                if ddl.ends_with(";") {
                    let _r =pool2.execute(sqlx::query(&ddl)).await;
                    // println!("插入数据之前执行sql内容\n:{} ",ddl);
                    ddl.clear();
                }
            }
            if insert_sqls.len()==batch_insert_szie || after >= f_len {
                let mut handles2 = Vec::with_capacity(batch_insert_szie);
                for sql in &insert_sqls {
                    handles2.push(tauri::async_runtime::spawn(insert((&sql).to_string(),pool2.clone())));
                }
                for handle in handles2 {
                    let _r = handle.await;
                }
                insert_sqls.clear();
            }
            if after >= f_len {
                break;
            }
        }
    } else {
        loop {
            let after = reader.stream_position().unwrap();
            if  after >= f_len {
                break
            }
            reader.read_line(&mut split_content).unwrap();
            if  split_content.trim().ends_with(";") {
                let _r = pool2.execute(sqlx::query(&split_content)).await;
                match _r {
                    Ok(_ok) => {
                    },
                    Err(_e) => {
                        info!("sql出错:{} ",&file);
                    }
                }
                // println!("执行sql内容:{}",split_content);
                split_content.clear();
            }
        }
    }
    // println!("{}文件导入结束",file);
}
async fn execute(table:  Table, pool2:MySqlPool ){
    // 先删除表结构，再执行ddl语句
    let drop_result = pool2.execute(sqlx::query(&format!("drop table if exists  {}",table.name))).await;
    match drop_result {
        Ok(_ok) => {
            let ddl_result = pool2.execute(sqlx::query(&table.ddl)).await;
            match  ddl_result {
                Ok(_ok) => {   
                    let name = table.name.clone();
                    let fields = table.fields.clone();
                    if table.data_files.len() == 0 {
                        info!("{}表数据处理完成,共0行,导入0行",name);
                    } else {
                        let mut handles2 = Vec::with_capacity(10);
                        for j in 0..table.data_files.len(){
                            handles2.push(execute_gz(table.data_files[j].clone(),name.clone(),fields.clone(),pool2.clone()));
                        }
                        for handle in handles2 {
                            let _r = handle.await;
                        }
                        let count_sql = format!("select count(1) as count from {}",table.name);
                        let row = sqlx::query(&count_sql).fetch_one(&pool2).await.unwrap();
                        let count:u32 = row.get_unchecked("count");
                        info!("{}表数据处理完成,共{}行,导入{}行",name,table.rows,count);
                    }
                },
                Err(_e) => {
                    info!("创建{}表sql出错:{:?} ",table.name,_e);
                }
            }
        },
        Err(_e) => {
            info!("删除{}表sql出错:{:?} ",table.name,_e);
        }
    }

    
}
async fn execute_gz (gz_file:PathBuf,table_name:String,table_fields:String,pool2:MySqlPool) {
    // 读取文本内容
    let f = File::open(gz_file.clone()).unwrap();
    let mut gz = GzDecoder::new(f);
    let mut data = String::new();
    gz.read_to_string(&mut data).unwrap();
    // 处理特殊字符 "\u{1e}"
    while data.contains("\u{1e}") {
        data = data.replace("\u{1e}", ",");
    }
    let insert_sql = format!("insert into {} ({}) values {} ",table_name,table_fields,data);
    let _r = pool2.execute(sqlx::query(&insert_sql)).await;
    match _r {
        Ok(_ok) => {
        },
        Err(_e) => {
            info!("插入数据文件{},到表{},出错:{} ",gz_file.display(),table_name,_e);
        }
    }
}
fn extract(file_path:&str )->Vec<Table>{
    // 创建文件夹 _nb3
    let nb_file = Path::new(file_path);
    let file = File::open(file_path).unwrap();
    let dir = env::temp_dir().join("_db_restore_").join(nb_file.file_stem().unwrap().to_str().unwrap());
    if dir.exists() {
        fs::remove_dir_all(dir.clone()).unwrap();
    } else {
        fs::create_dir_all(dir.clone()).unwrap();
    }
    let _a = Archive::new(file).unpack(dir.clone()).unwrap();
    let mut tables = Vec::new();
    // let meta_path = Path::new("./_nb3/meta.json");
    let mut meta_path = dir.clone();
    meta_path.push("meta.json");
    let v: Value = serde_json::from_str(&fs::read_to_string(meta_path).unwrap()).unwrap();
    let objects = v["Objects"].as_array().unwrap();
    for obj in objects {
        if obj["Type"].as_str().unwrap() == "Table" {
            let name = obj["Name"].as_str().unwrap().to_string();
            let rows= obj["Rows"].as_str().unwrap().to_string().parse().unwrap();
            let meta = obj["Metadata"]["Filename"].as_str().unwrap().to_string();
            // 处理数据文件
            // let table_meta = format!("./_nb3/{}",meta);
            let table_meta = dir.clone().join(meta);
            // println!("处理表{} JSON信息 对应gz文件{}",name,table_meta.display());
            let tar_gz = File::open(table_meta).unwrap();
            let mut gz = GzDecoder::new(tar_gz);
            let mut s = String::new();
            gz.read_to_string(&mut s).unwrap();
            let t: Value = serde_json::from_str(&s).unwrap();
            let ddl = t["DDL"].as_str().unwrap().to_string();
            let mut data_files = Vec::new();
            let data = t["Data"].as_array().unwrap();
            for d in data {
                data_files.push(dir.clone().join(d["Filename"].as_str().unwrap().to_string()));
            }
            let fields_array = t["Fields"].as_array().unwrap();
            let mut fields_vec = Vec::new();
            for fa in fields_array {
                fields_vec.push(fa.as_str().unwrap().to_string());
            }
            let fields = fields_vec.join(",");
            let table = Table{
                name,
                rows,
                ddl,
                fields,
                data_files
            };  
            tables.push(table);
        }
    }
    return tables;
}
// 从单一sql文件分割成多个sql文件
fn split(file_path:&str )->Vec<String>{
    // 创建文件夹 _nb3
    let nb_file = Path::new(file_path);
    let dir = env::temp_dir().join("_db_restore_").join(nb_file.file_stem().unwrap().to_str().unwrap());
    if dir.exists() {
        fs::remove_dir_all(dir.clone()).unwrap();
    } else {
        fs::create_dir_all(dir.clone()).unwrap();
    }
    // fs::create_dir_all("_nb3").unwrap();
    let file = File::open(file_path).unwrap();
    let f_len = file.metadata().unwrap().len();
    let mut f = BufReader::new(file);
    let mut w = BufWriter::new(File::create(dir.clone().join("tmp.txt")).unwrap());
    let mut tables = Vec::new();
    loop {
        let after = f.stream_position().unwrap();
        if  after >= f_len {
            w.flush().unwrap();
            break
        }
        let mut split_content = String::from("");
        f.read_line(&mut split_content).unwrap();
        if split_content.starts_with("--") {
            continue;
        }
        if split_content.starts_with("DROP TABLE IF EXISTS") || split_content.starts_with("DROP PROCEDURE") {
            w.flush().unwrap();
            let v:Vec<&str> = split_content.split('`').collect();
            let file_name = dir.clone().join(format!("{}.sql",v[1]));
            w = BufWriter::new(File::create(&file_name).unwrap());
            if split_content.starts_with("DROP TABLE IF EXISTS") {
                tables.push(file_name.to_str().unwrap().to_string());
            }
        }
        w.write(split_content.as_bytes()).unwrap();
    }
    return tables;
}