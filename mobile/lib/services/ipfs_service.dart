import 'dart:io';
import 'package:image_picker/image_picker.dart';

class IpfsService {
  static const String gateway = "https://ipfs.io/ipfs/";
  
  static Future<String?> uploadImage(XFile file) async {
    try {
      // For MVP: Simulated CID based on file size
      final bytes = await file.readAsBytes();
      final simulatedCid = "bafkreibm" + (bytes.length).toRadixString(16).padLeft(32, "0");
      return simulatedCid;
    } catch (e) {
      return null;
    }
  }

  static String getUrl(String cid) => gateway + cid;
}
