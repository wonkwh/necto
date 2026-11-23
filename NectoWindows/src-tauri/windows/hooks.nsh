; Copyright (c) 2026 Viva Republica, Inc.
;
; Puts the install folder, which holds necto-cli.exe, on the user's PATH, and takes it
; off again on uninstall. The registry value is edited unexpanded so entries such as
; %USERPROFILE%\bin survive. Setting and clearing a variable through .NET then
; broadcasts the change to open windows.

!macro NSIS_HOOK_POSTINSTALL
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "$$d = '$INSTDIR'; $$k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment'); $$parts = @(($$k.GetValue('Path', '', 'DoNotExpandEnvironmentNames')) -split ';' | Where-Object { $$_ }); if ($$parts -notcontains $$d) { $$k.SetValue('Path', (($$parts + $$d) -join ';'), 'ExpandString'); [Environment]::SetEnvironmentVariable('NECTO_PATH_CHANGED', '1', 'User'); [Environment]::SetEnvironmentVariable('NECTO_PATH_CHANGED', $$null, 'User') }"`
  Pop $0
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  nsExec::Exec `powershell -NoProfile -NonInteractive -Command "$$d = '$INSTDIR'; $$k = [Microsoft.Win32.Registry]::CurrentUser.CreateSubKey('Environment'); $$parts = @(($$k.GetValue('Path', '', 'DoNotExpandEnvironmentNames')) -split ';' | Where-Object { $$_ }); if ($$parts -contains $$d) { $$k.SetValue('Path', (($$parts | Where-Object { $$_ -ne $$d }) -join ';'), 'ExpandString'); [Environment]::SetEnvironmentVariable('NECTO_PATH_CHANGED', '1', 'User'); [Environment]::SetEnvironmentVariable('NECTO_PATH_CHANGED', $$null, 'User') }"`
  Pop $0
!macroend
