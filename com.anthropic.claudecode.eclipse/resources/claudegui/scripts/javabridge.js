/* javabridge.js — the view's Java functions, as the rest of the page calls them:
   window._name(...). Loaded first, before every other script.

   The view gives the page ONE BrowserFunction, _java(name, ...args), and this script
   defines each of the others as a call through it (ClaudeGuiView.JavaBridgeFunction).
   On WebKitGTK (Linux, FreeBSD) every BrowserFunction makes SWT rebuild and re-inject the
   whole set, and with ~95 of them the first page load could stall for good: the view
   stayed blank until it was reopened. One function never does.

   Edge (Windows) and macOS WebKit define _java before this page's scripts run, so every
   window._name exists from the first line on, as the functions themselves did. WebKitGTK
   defines it only once the page has been read; init.js (ccBootIfReady) installs them
   then, before the boot. */

let ccJavaBridged = false;

/* Defines window._name for every function the view made, once _java is there. True once
   they are all defined. */
function ccInstallJavaBridge() {
  if (ccJavaBridged) return true;
  if (typeof window._java !== 'function') return false;
  let names = null;
  try { names = JSON.parse(window._java('_pageFunctions')); } catch (e) { return false; }
  if (!Array.isArray(names)) return false;
  names.forEach(name => {
    window[name] = function () {
      return window._java.apply(null, [name].concat(Array.prototype.slice.call(arguments)));
    };
  });
  ccJavaBridged = true;
  return true;
}

ccInstallJavaBridge();
