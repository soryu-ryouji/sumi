//! 领域核心：存储底座、索引流水线、解析器、事件总线。

pub mod config;
pub mod fulltext;
pub mod cover;
pub mod index;
pub mod item;
pub mod locks;
pub mod registry_file;
pub mod metadata;
pub mod metadata_store;
pub mod tasks;
pub mod events;
pub mod paths;
pub mod parser;
pub mod pipeline;
pub mod scanner;
pub mod watcher;
pub mod startup;
