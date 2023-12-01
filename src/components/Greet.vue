<script setup>
import { ref, onMounted } from "vue";
import { FolderOpened,Coin } from '@element-plus/icons-vue'

const file_path = ref("E:\\work\\dagl\\hongyan\\数据\\20231201100202.nb3");
const url = ref("mysql://hams:hams@127.0.0.1:3306/hongyan");
const working = ref(false);
const status = ref("");
const percentage = ref(0);

onMounted(()=>{
  console.log(window.__TAURI__)
  window.__TAURI__.event.listen("tauri://file-drop", (event) => {
      file_path.value = event.payload[0];
  });
  // 监听事件
  window.__TAURI__.event.listen('percentage', (event) => {
    console.log(event);
    if(event.payload && working) {
      percentage.value = event.payload.percentage/100;
      status.value = '正在还原...' + event.payload.msg;
    }
  });

})
function restore() {
  if (!url.value){
    alert("请输入mysql连接字符串");
    return;
  }
  if (!file_path.value){
    alert("请选择备份文件");
    return;
  }
  working.value = true;
  status.value = '正在还原...'
  window.__TAURI__.tauri.invoke("restore", { filePath: file_path.value, url: url.value })
    .then((r) => {
      console.log(r);
      working.value = false;
      status.value = r;
    })
    .catch((e) => {
      console.log(e);
      working.value = false;
      status.value = e;
    });
}
function pickFile() {
  window.__TAURI__.dialog.open({
    title: "打开备份文件",
    directory: false,
    multiple: false,
    filters: [{ name: "mysql备份文件", extensions: ["nb3", "sql"] }],
  }).then((f) => {
    if (f) {
      file_path.value = f;
    }
  });
}
</script>

<template>
  <el-card class="box-card">
    <template #header>
      <div class="card-header">
        <h2>支持sql文件或者nb3</h2>
        <div>
          <el-button class="button" type="primary" @click="restore()" :icon="Coin" :disabled="working">开始还原</el-button>
        </div>
      </div>
    </template>
    <el-space direction="vertical" :size="30" style="padding-top: 0;">
      <div>
        <el-input  size="large" v-model="file_path" placeholder="通过拖拽或者点击选择,指定要导入的文件" style="min-width: 600px;" >
          <template #prepend>导入文件路径:</template>
          <template #append>
            <el-button type="info" @click="pickFile()"  :icon="FolderOpened" :disabled="working">打开</el-button>
          </template>
        </el-input>
      </div>
      <div>
        <el-input  size="large" v-model="url" placeholder="输入mysql连接字符串" style="min-width: 600px;" >
          <template #prepend>数据库连接串:</template>
        </el-input>
      </div>
      <div  style="width:100%">
        <el-text >{{ status }}</el-text>
        <el-progress :percentage="percentage" v-show="working"/>
          <!-- <el-button class="button" type="info" @click="pickFile()"  :icon="FolderOpened" :disabled="working">选择文件</el-button>
          <el-button class="button" type="primary" @click="restore()" :icon="Coin" :disabled="working">开始还原</el-button> -->
      </div>
    </el-space>
  </el-card>
</template>
<style>
.card-header {
  display: flex;
  justify-content: space-between;
  align-items: baseline;
}

.text {
  font-size: 14px;
}

.item {
  margin-bottom: 18px;
}

.box-card {
  width:100%;
  margin: 0;
  height: 90%;
}
</style>
