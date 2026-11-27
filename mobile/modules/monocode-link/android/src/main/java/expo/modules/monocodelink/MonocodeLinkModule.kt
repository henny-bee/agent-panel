package expo.modules.monocodelink

import expo.modules.kotlin.functions.Coroutine
import expo.modules.kotlin.modules.Module
import expo.modules.kotlin.modules.ModuleDefinition
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

class MonocodeLinkModule : Module() {
  companion object { init { System.loadLibrary("monocode_mobile_link") } }
  private external fun nativeCall(input: String): String
  override fun definition() = ModuleDefinition {
    Name("MonocodeLink")
    // Each call blocks on the network, a watch's for up to its wait. Expo
    // runs async functions on one thread, where that wait would hold every
    // other call, such as opening the next chat, behind it.
    AsyncFunction("execute") Coroutine { input: String ->
      withContext(Dispatchers.IO) { nativeCall(input) }
    }
  }
}
