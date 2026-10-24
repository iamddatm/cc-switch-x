import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./styles.css";

// 桌面应用不需要 WebView 自带的浏览器右键菜单（刷新、打印、检查元素等）。
document.addEventListener("contextmenu", (event) => event.preventDefault());

ReactDOM.createRoot(document.getElementById("root")!).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);
