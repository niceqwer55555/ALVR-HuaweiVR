# 未包含：华为 OpenXR 私有 fork

上游 Android 客户端 crate 使用该 fork，但它未在本仓库公开，无法从公开源重建。
服务端/Dashboard 不依赖它；根 workspace 已排除 client crates，因此不影响服务端构建。
如需编译 Android 客户端，请向原作者获取该目录后放回此处。
