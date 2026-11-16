; QuarterMaster installer hooks. They are inserted inside the Install and Uninstall sections of
; installer.nsi, so they can use its variables ($QmInstallCli, $QmAddPath,
; $DeleteAppDataCheckboxState, $UpdateMode).

!macro NSIS_HOOK_POSTINSTALL
  ; QuarterMaster keeps its data (qm-data), its mod workspace and its own updates in the install
  ; folder, so the Users group (S-1-5-32-545) gets Modify on it. Without this a normal account
  ; could not write beside the program in Program Files.
  nsExec::ExecToLog 'icacls "$INSTDIR" /grant *S-1-5-32-545:(OI)(CI)M /T /C /Q'

  ; The command-line tool is copied with the other files; it goes again when it was not wanted.
  ; "" (a silent or passive install) counts as wanted.
  ${If} $QmInstallCli == 0
    Delete "$INSTDIR\qm.exe"
  ${EndIf}
  ${If} $QmInstallCli != 0
  ${AndIf} $QmAddPath != 0
    nsExec::ExecToLog '"$INSTDIR\qm-launcher.exe" --path-add'
  ${Else}
    nsExec::ExecToLog '"$INSTDIR\qm-launcher.exe" --path-remove'
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; While the program still exists, it takes its own files out of the game folder (its winmm.dll
  ; and the Mods link). With "delete my data" ticked it also deletes qm-data, without following
  ; links and never the mod workspace. An update keeps everything.
  ${If} ${FileExists} "$INSTDIR\qm-launcher.exe"
    ${If} $UpdateMode <> 1
      nsExec::ExecToLog '"$INSTDIR\qm-launcher.exe" --path-remove'
      ${If} $DeleteAppDataCheckboxState = 1
        nsExec::ExecToLog '"$INSTDIR\qm-launcher.exe" --remove-data'
      ${Else}
        nsExec::ExecToLog '"$INSTDIR\qm-launcher.exe" --restore-game'
      ${EndIf}
    ${EndIf}
  ${EndIf}
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; What a launcher update leaves beside the program. The mod workspace (mod-workspace by
  ; default) is not touched, so the install folder stays when it holds one.
  ${If} $UpdateMode <> 1
    Delete "$INSTDIR\qm-launcher.old.exe"
    Delete "$INSTDIR\qm_winmm.old.dll"
    RMDir "$INSTDIR"
  ${EndIf}
!macroend
