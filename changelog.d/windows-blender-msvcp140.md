### Fixed

- Windows: the Python wheel no longer crashes Blender 4.5 (an access violation in
  `MSVCP140.dll`) when it encodes or decodes KTX2 textures, for example in `decode_tile` or a
  textured `mesh_to_tileset`. The bundled Basis Universal C++ was built with the newer runtime's
  constexpr `std::mutex`, which Blender 4.5's older bundled C++ runtime cannot lock; it now keeps
  the runtime-constructed mutex that every `MSVCP140.dll` supports.
